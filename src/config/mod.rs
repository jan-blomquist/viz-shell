//! Configurations: YAML documents in `*.vz.yml` files, named, chained by
//! `extends`, folded into the [`Effective`] configuration vz runs with. Each
//! step is a module with a pure function; [`Files`] is the only door to the
//! filesystem.
//!
//! 1. [`scan`]: the library folder, the folders it lists under `scan:`, the
//!    repository root, the `-f` files: their configuration files, read.
//! 2. [`parse`]: each file's documents, each checked on its own.
//! 3. [`paths`]: their host paths made absolute.
//! 4. [`name`]: each document named; one nameless per scope is `default`.
//! 5. [`configs`]: the configurations by scope and name, one per name.
//! 6. [`configs`]: the chain of the one asked for.
//! 7. [`effective`]: the chain folded, with [`merge`].
//!
//! [`chain`] renders the chain for `--show-effective-config`, the
//! container's `vz.chain` label and the banner.

mod chain;
mod configs;
mod effective;
mod merge;
mod name;
mod parse;
mod paths;
mod scan;
mod template;

use std::path::{Path, PathBuf};

use anyhow::Context;

use name::Document;
use scan::Scanned;

pub use chain::{CUT, Link, banner_line, header, links};
pub use configs::{ConfigInfo, Step, Table};
pub use effective::{
    Effective, EffectiveEnv, EffectiveHooks, EnvFile, ImageLayer, MountEntry, StateEntry,
};
pub use parse::{BuildSpec, ImageSource, MountMode, StateKind, is_env_name, with_default_tag};
pub use paths::{expand_path, resolve_host_path, shown, substitute, tilde};
pub use scan::Source;
pub use template::{needs_scaffold, scaffold};

/// The filesystem as the scan sees it.
pub trait Files {
    /// The names of the files in `dir`; none when it does not exist.
    fn list(&self, dir: &Path) -> anyhow::Result<Vec<String>>;
    fn read(&self, path: &Path) -> anyhow::Result<String>;
}

/// The host's filesystem.
pub struct RealFiles;

impl Files for RealFiles {
    fn list(&self, dir: &Path) -> anyhow::Result<Vec<String>> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).with_context(|| format!("listing {}", dir.display())),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.with_context(|| format!("listing {}", dir.display()))?;
            if entry.path().is_file() {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        Ok(names)
    }

    fn read(&self, path: &Path) -> anyhow::Result<String> {
        Ok(std::fs::read_to_string(path)?)
    }
}

/// What to load.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub home: PathBuf,
    pub repo_root: PathBuf,
    pub library_dir: PathBuf,
    /// `-f` files, absolute: repository files wherever they are.
    pub extra_files: Vec<PathBuf>,
    /// `-c`; `default` without it.
    pub config: Option<String>,
}

impl Request {
    fn folders(&self) -> scan::Folders<'_> {
        scan::Folders {
            library: &self.library_dir,
            repo_root: &self.repo_root,
            extra_files: &self.extra_files,
            home: &self.home,
        }
    }
}

/// The configuration of one run.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub effective: Effective,
    /// The configurations applied, in fold order.
    pub chain: Vec<Step>,
    /// Every configuration file scanned, in scan order.
    pub files: Vec<Source>,
}

impl Loaded {
    /// The files of the chain, each once, in fold order.
    pub fn chain_files(&self) -> Vec<Source> {
        let mut files: Vec<Source> = Vec::new();
        for step in &self.chain {
            if !files.contains(&step.file) {
                files.push(step.file.clone());
            }
        }
        files
    }
}

/// The pipeline: every configuration read, then the chain of the one asked
/// for, folded.
pub fn load(request: &Request, files: &impl Files) -> anyhow::Result<Loaded> {
    let table = read_configs(request, files)?;
    let chain = configs::resolve_chain(&table, request.config.as_deref())?;
    let effective = effective::fold(&chain)?;
    Ok(Loaded {
        effective,
        chain: chain.into_iter().map(Step::of).collect(),
        files: table.files,
    })
}

/// Steps 1 to 5: every file scanned, its documents read, then named, then
/// keyed by scope and name.
pub fn read_configs(request: &Request, files: &impl Files) -> anyhow::Result<Table> {
    let scan = scan::collect(&request.folders(), files)?;
    let mut documents = Vec::new();
    for file in &scan.files {
        let read = read_file(file, request)
            .with_context(|| format!("in {}", file.source.path.display()))?;
        documents.extend(read);
    }
    Ok(Table {
        configs: configs::by_name(name::name(documents)?)?,
        searched: scan.searched,
        files: scan.files.into_iter().map(|file| file.source).collect(),
    })
}

/// Steps 2 and 3 for one file: its documents parsed, their paths resolved.
fn read_file(file: &Scanned, request: &Request) -> anyhow::Result<Vec<Document>> {
    let mut documents = Vec::new();
    for mut layer in parse::parse(&file.text)? {
        paths::resolve(
            &mut layer,
            file.source.dir(),
            &request.home,
            &request.repo_root,
        )?;
        documents.push(Document {
            source: file.source.clone(),
            layer,
        });
    }
    Ok(documents)
}

/// Every configuration, for `vz configs`: the repository's first.
pub fn list(table: &Table) -> Vec<ConfigInfo> {
    configs::list(table)
}

/// Fakes and factories the steps' tests share.
#[cfg(test)]
pub(crate) mod testing {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use anyhow::Context;

    use super::configs::{self, Table};
    use super::effective::{self, Effective};
    use super::name::{self, Document};
    use super::parse;
    use super::scan::{Scope, Source};
    use super::{Files, Loaded, Request};

    pub const HOME: &str = "/home/sally";
    pub const REPO: &str = "/home/sally/repos/app";
    pub const LIBRARY: &str = "/home/sally/.config/viz-shell";

    /// Files in memory: each path with its text.
    pub struct FakeFiles {
        files: BTreeMap<PathBuf, String>,
    }

    impl FakeFiles {
        pub fn with(files: &[(&str, &str)]) -> Self {
            Self {
                files: files
                    .iter()
                    .map(|(path, text)| (PathBuf::from(path), text.to_string()))
                    .collect(),
            }
        }
    }

    impl Files for FakeFiles {
        fn list(&self, dir: &Path) -> anyhow::Result<Vec<String>> {
            Ok(self
                .files
                .keys()
                .filter(|path| path.parent() == Some(dir))
                .filter_map(|path| Some(path.file_name()?.to_string_lossy().into_owned()))
                .collect())
        }

        fn read(&self, path: &Path) -> anyhow::Result<String> {
            self.files
                .get(path)
                .cloned()
                .with_context(|| format!("no file {}", path.display()))
        }
    }

    /// Sally's run in `~/repos/app`, with these `-f` files.
    pub fn request(config: Option<&str>, extra_files: &[&str]) -> Request {
        Request {
            home: PathBuf::from(HOME),
            repo_root: PathBuf::from(REPO),
            library_dir: PathBuf::from(LIBRARY),
            extra_files: extra_files.iter().map(PathBuf::from).collect(),
            config: config.map(str::to_owned),
        }
    }

    /// The configurations of these files, by path.
    pub fn table(files: &[(&str, &str)]) -> anyhow::Result<Table> {
        super::read_configs(&request(None, &[]), &FakeFiles::with(files))
    }

    /// The whole pipeline over these files.
    pub fn load(files: &[(&str, &str)], config: Option<&str>) -> anyhow::Result<Loaded> {
        super::load(&request(config, &[]), &FakeFiles::with(files))
    }

    /// The message of `result`'s error, with its context.
    pub fn message<T: std::fmt::Debug>(result: anyhow::Result<T>) -> String {
        format!("{:#}", result.unwrap_err())
    }

    /// The repository's `app.vz.yml`.
    pub fn repository_file() -> Source {
        Source {
            path: PathBuf::from(REPO).join("app.vz.yml"),
            scope: Scope::Repository,
        }
    }

    /// `text` as the repository's `app.vz.yml`, alone, its paths as
    /// written: parsed, named, keyed, chained and folded.
    pub fn fold_text(text: &str, config: Option<&str>) -> anyhow::Result<Effective> {
        let documents = parse::parse(text)?
            .into_iter()
            .map(|layer| Document {
                source: repository_file(),
                layer,
            })
            .collect();
        let table = Table {
            configs: configs::by_name(name::name(documents)?)?,
            searched: vec![PathBuf::from(REPO)],
            files: vec![repository_file()],
        };
        let chain = configs::resolve_chain(&table, config)?;
        effective::fold(&chain)
    }

    pub fn effective(text: &str, config: Option<&str>) -> Effective {
        fold_text(text, config).unwrap_or_else(|error| panic!("{error:#}"))
    }

    /// Why `text`, as the repository's `app.vz.yml`, fails for its default.
    pub fn error(text: &str) -> String {
        message(fold_text(text, None))
    }
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    /// A file of `examples/`.
    fn example(path: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join(path);
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    /// Files by their path below a home, each with its text.
    type Layout = Vec<(String, String)>;

    /// The file of `examples/` at `example`, placed at `path` below a home.
    fn at(path: &str, example_path: &str) -> (String, String) {
        (path.to_owned(), example(example_path))
    }

    /// A file of `text` at `path` below a home.
    fn written(path: &str, text: &str) -> (String, String) {
        (path.to_owned(), text.to_owned())
    }

    /// A throwaway home holding `files`, each at its path below it.
    fn home_with(files: &[(String, String)]) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = home.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        home
    }

    /// A run in `~/repos/app` of the home, from its library.
    fn request_in(home: &Path, config: Option<&str>) -> Request {
        Request {
            home: home.to_owned(),
            repo_root: home.join("repos/app"),
            library_dir: home.join(".config/viz-shell"),
            extra_files: vec![],
            config: config.map(str::to_owned),
        }
    }

    /// `--show-effective-config`'s lines above the YAML.
    fn header_of(home: &Path, config: Option<&str>) -> anyhow::Result<Vec<String>> {
        let request = request_in(home, config);
        let loaded = load(&request, &RealFiles)?;
        let images = crate::build::describe(&loaded.effective.images, &request.repo_root, home)?;
        Ok(header(&loaded.chain, &images, &request.repo_root, home))
    }

    fn lines(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    fn library_example() -> Layout {
        vec![
            at(
                ".config/viz-shell/vz-debian-trixie.vz.yml",
                "library/library/vz-debian-trixie.vz.yml",
            ),
            at(
                ".config/viz-shell/trusted.vz.yml",
                "library/library/trusted.vz.yml",
            ),
        ]
    }

    fn local_example() -> Layout {
        vec![
            at("repos/app/default.vz.yml", "local/default.vz.yml"),
            at("repos/app/sally.vz.yml", "local/sally.vz.yml"),
            at("repos/app/sally.Dockerfile", "local/sally.Dockerfile"),
        ]
    }

    fn configs_example() -> Layout {
        vec![
            at("repos/app/default.vz.yml", "configs/default.vz.yml"),
            at("repos/app/debian12.vz.yml", "configs/debian12.vz.yml"),
            at("repos/app/trusted.vz.yml", "configs/trusted.vz.yml"),
            at(
                ".config/viz-shell/vz-debian-trixie.vz.yml",
                "configs/library/vz-debian-trixie.vz.yml",
            ),
            at(
                ".config/viz-shell/trusted.vz.yml",
                "configs/library/trusted.vz.yml",
            ),
        ]
    }

    fn image_stack_example() -> Layout {
        vec![
            at("repos/app/default.vz.yml", "image-stack/default.vz.yml"),
            at("repos/app/deep.vz.yml", "image-stack/deep.vz.yml"),
            at("repos/app/pulled.vz.yml", "image-stack/pulled.vz.yml"),
            at(
                "repos/app/bare-tools.vz.yml",
                "image-stack/bare-tools.vz.yml",
            ),
            at("repos/app/base.Dockerfile", "image-stack/base.Dockerfile"),
            at("repos/app/tools.Dockerfile", "image-stack/tools.Dockerfile"),
            at("repos/app/alone.Dockerfile", "image-stack/alone.Dockerfile"),
        ]
    }

    const LIBRARY_BASE: &str = "# vz-debian-trixie (~/.config/viz-shell/vz-debian-trixie.vz.yml)";
    const LIBRARY_TRUSTED: &str = "# trusted (~/.config/viz-shell/trusted.vz.yml)";
    const LIBRARY_IMAGE: &str = "# image: debian:stable-slim (vz-debian-trixie, \
                                 ~/.config/viz-shell/vz-debian-trixie.vz.yml)";

    #[test]
    fn load__each_example_s_layout__its_chain_and_images() {
        let cases: Vec<(&str, Layout, Option<&str>, Vec<String>)> = vec![
            (
                "library: -c runs a library configuration by name",
                library_example(),
                Some("vz-debian-trixie"),
                lines(&[LIBRARY_BASE, LIBRARY_IMAGE]),
            ),
            (
                "library: -c trusted without the repository's own, the library's",
                library_example(),
                Some("trusted"),
                lines(&[LIBRARY_BASE, LIBRARY_TRUSTED, LIBRARY_IMAGE]),
            ),
            (
                "library: ci on the repository's trusted, on the library's",
                [
                    library_example(),
                    vec![at("repos/app/app.vz.yml", "library/app.vz.yml")],
                ]
                .concat(),
                Some("ci"),
                lines(&[
                    LIBRARY_BASE,
                    LIBRARY_TRUSTED,
                    "# trusted (app.vz.yml)",
                    "# ci (app.vz.yml)",
                    LIBRARY_IMAGE,
                ]),
            ),
            (
                "pull-image: one nameless configuration, the default",
                vec![at("repos/app/app.vz.yml", "pull-image/app.vz.yml")],
                None,
                lines(&[
                    "# default (app.vz.yml)",
                    "# image: debian:stable-slim (default, app.vz.yml)",
                ]),
            ),
            (
                "local: sally's configuration on the default, her Dockerfile stacked",
                local_example(),
                Some("sally"),
                lines(&[
                    "# default (default.vz.yml)",
                    "# sally (sally.vz.yml)",
                    "# image: debian:stable-slim (default, default.vz.yml)",
                    "# image: sally.Dockerfile (sally, sally.vz.yml, ARG BASE: required, stacks)",
                ]),
            ),
            (
                "configs: a file of its own extends default",
                configs_example(),
                Some("debian12"),
                lines(&[
                    "# default (default.vz.yml)",
                    "# debian12 (debian12.vz.yml)",
                    "# image: debian:stable-slim (default, default.vz.yml)",
                    "# image: debian:bookworm-slim (debian12, debian12.vz.yml, replaces)",
                ]),
            ),
            (
                "configs: trusted extends the library's trusted",
                configs_example(),
                Some("trusted"),
                lines(&[
                    LIBRARY_BASE,
                    LIBRARY_TRUSTED,
                    "# trusted (trusted.vz.yml)",
                    LIBRARY_IMAGE,
                ]),
            ),
            (
                "image-stack: stacks, then replaces",
                image_stack_example(),
                Some("deep"),
                lines(&[
                    "# default (default.vz.yml)",
                    "# tools (default.vz.yml)",
                    "# deep (deep.vz.yml)",
                    "# image: base.Dockerfile (default, default.vz.yml)",
                    "# image: tools.Dockerfile (tools, default.vz.yml, ARG BASE: stacks)",
                    "# image: alone.Dockerfile (deep, deep.vz.yml, replaces)",
                ]),
            ),
            (
                "image-stack: stacks on a reference",
                image_stack_example(),
                Some("pulled-tools"),
                lines(&[
                    "# pulled (pulled.vz.yml)",
                    "# pulled-tools (pulled.vz.yml)",
                    "# image: debian:stable-slim (pulled, pulled.vz.yml)",
                    "# image: tools.Dockerfile (pulled-tools, pulled.vz.yml, ARG BASE: stacks)",
                ]),
            ),
            (
                "image-stack: extends nothing, its default below",
                image_stack_example(),
                Some("bare-tools"),
                lines(&[
                    "# bare-tools (bare-tools.vz.yml)",
                    "# image: tools.Dockerfile (bare-tools, bare-tools.vz.yml, ARG BASE: its default)",
                ]),
            ),
            (
                "library: an image under the home, written with ~",
                vec![
                    written(
                        ".config/viz-shell/tools.vz.yml",
                        "name: tools\n\
                         image: { dockerfile: ~/repos/tools/Dockerfile, context: ~/repos/tools }\n",
                    ),
                    written("repos/tools/Dockerfile", "FROM debian:stable-slim\n"),
                ],
                Some("tools"),
                lines(&[
                    "# tools (~/.config/viz-shell/tools.vz.yml)",
                    "# image: ~/repos/tools/Dockerfile (tools, ~/.config/viz-shell/tools.vz.yml)",
                ]),
            ),
        ];
        for (case, files, config, expected) in cases {
            let home = home_with(&files);

            let header =
                header_of(home.path(), config).unwrap_or_else(|error| panic!("{case}: {error:#}"));

            assert_eq!(header, expected, "{case}");
        }
    }

    /// A file of a case of `examples/resolution`, at the same name in the
    /// repository.
    fn resolution(case: &str, file_name: &str) -> (String, String) {
        at(
            &format!("repos/app/{file_name}"),
            &format!("resolution/{case}/{file_name}"),
        )
    }

    #[test]
    fn load__each_resolution_example__refused_with_its_message() {
        let two_folders = vec![
            written(".config/viz-shell/default.vz.yml", "scan: [~/one, ~/two]\n"),
            at(
                "one/trusted.vz.yml",
                "resolution/two-folders/one/trusted.vz.yml",
            ),
            at(
                "two/trusted.vz.yml",
                "resolution/two-folders/two/trusted.vz.yml",
            ),
        ];
        let no_default = vec![written(
            ".config/viz-shell/vz-debian-trixie.vz.yml",
            "name: vz-debian-trixie\nimage: debian:stable-slim\n",
        )];
        let cases: Vec<(&str, Layout, Option<&str>, &str)> = vec![
            (
                "unknown",
                vec![resolution("unknown", "ci.vz.yml")],
                Some("ci"),
                "configuration `ci` extends `nope`: no configuration `nope`",
            ),
            (
                "unknown, folders and names",
                vec![resolution("unknown", "ci.vz.yml")],
                Some("ci"),
                "/.config/viz-shell, ",
            ),
            (
                "cycle",
                vec![
                    resolution("cycle", "a.vz.yml"),
                    resolution("cycle", "b.vz.yml"),
                ],
                Some("a"),
                "configurations extend each other in a cycle: a → b → a",
            ),
            (
                "two names",
                vec![resolution("two-names", "app.vz.yml")],
                None,
                "extends takes one name",
            ),
            (
                "profiles: refused",
                vec![resolution("profiles-key", "app.vz.yml")],
                None,
                "`profiles:` is no more",
            ),
            (
                "two nameless",
                vec![
                    resolution("two-nameless", "a.vz.yml"),
                    resolution("two-nameless", "b.vz.yml"),
                ],
                None,
                "two configurations without `name:` in the repository: in ",
            ),
            (
                "one name twice",
                vec![
                    resolution("duplicate-name", "a.vz.yml"),
                    resolution("duplicate-name", "b.vz.yml"),
                ],
                None,
                "two configurations named `ci` in the repository: in ",
            ),
            (
                "nameless beside one named default",
                vec![
                    resolution("nameless-and-default", "a.vz.yml"),
                    resolution("nameless-and-default", "b.vz.yml"),
                ],
                None,
                "two configurations named `default` in the repository",
            ),
            (
                "ARG BASE without a default, nothing before it",
                vec![
                    resolution("required-base", "app.vz.yml"),
                    resolution("required-base", "overlay.Dockerfile"),
                ],
                None,
                "`overlay.Dockerfile` requires BASE: no image before it in the chain",
            ),
            (
                "scan outside the library",
                vec![resolution("scan-outside", "app.vz.yml")],
                None,
                "`scan` belongs in a file of",
            ),
            (
                "one library name in two folders",
                two_folders,
                None,
                "two configurations named `trusted` in the library: in ",
            ),
            (
                "no default",
                no_default,
                None,
                "no default configuration: add vz.yml with `extends: vz-debian-trixie`, or run with -c NAME",
            ),
        ];
        for (case, files, config, expected) in cases {
            let home = home_with(&files);

            let message = format!("{:#}", header_of(home.path(), config).unwrap_err());

            assert!(message.contains(expected), "{case}: {message}");
        }
    }

    /// The host's files, but at `root` only those git tracks: the files a
    /// developer keeps there for themselves are not this repository's.
    struct TrackedAt<'a>(&'a Path);

    impl Files for TrackedAt<'_> {
        fn list(&self, dir: &Path) -> anyhow::Result<Vec<String>> {
            let names = RealFiles.list(dir)?;
            Ok(names
                .into_iter()
                .filter(|file_name| dir != self.0 || crate::repo::is_tracked(&dir.join(file_name)))
                .collect())
        }

        fn read(&self, path: &Path) -> anyhow::Result<String> {
            RealFiles.read(path)
        }
    }

    /// This repository, on a library its templates wrote, run as `config`:
    /// its chain line and its images above the library's base.
    fn this_repository(config: Option<&str>) -> (String, Vec<String>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let library = tempfile::tempdir().unwrap();
        scaffold(library.path()).unwrap();
        let home = Path::new("/home/sally");
        let request = Request {
            home: home.to_owned(),
            repo_root: root.to_owned(),
            library_dir: library.path().to_owned(),
            extra_files: vec![],
            config: config.map(str::to_owned),
        };
        let loaded = load(&request, &TrackedAt(root)).unwrap();
        let chain = banner_line(&links(&loaded.chain, root, home));
        let mut images = crate::build::describe(&loaded.effective.images, root, home).unwrap();
        images.remove(0);
        (chain, images)
    }

    /// `default` on vz-debian-trixie, `dev` on `default`.
    #[test]
    fn load__this_repository__default_and_dev_on_the_library_s_base() {
        let cases = [
            (None, "vz-debian-trixie (library) → default"),
            (Some("dev"), "vz-debian-trixie (library) → default → dev"),
        ];
        for (config, expected) in cases {
            let (chain, _) = this_repository(config);

            assert_eq!(chain, expected, "{config:?}");
        }
    }

    /// Each Dockerfile requires the image below it.
    #[test]
    fn load__this_repository__each_dockerfile_stacks_on_the_one_below() {
        let cases: [(Option<&str>, &[&str]); 2] = [
            (
                None,
                &["Dockerfile (default, vz.yml, ARG BASE: required, stacks)"],
            ),
            (
                Some("dev"),
                &[
                    "Dockerfile (default, vz.yml, ARG BASE: required, stacks)",
                    "dev.Dockerfile (dev, dev.vz.yml, ARG BASE: required, stacks)",
                ],
            ),
        ];
        for (config, expected) in cases {
            let (_, images) = this_repository(config);

            assert_eq!(images, expected, "{config:?}");
        }
    }

    /// The recipes in `examples/`, this repository's own configuration and
    /// the library a first run writes stay valid as the schema changes: each
    /// folder as a repository root, with its `library/` when it has one, else
    /// the written library. Every configuration loads.
    ///
    /// A deliberate sweep over files on disk, loops included: the inputs are
    /// whatever `examples/` holds, so no table could list them. An addition
    /// to the focused tests above, never a replacement.
    #[test]
    fn load__every_example_and_this_repository__every_configuration() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let files = TrackedAt(root);
        let written = tempfile::tempdir().unwrap();
        scaffold(written.path()).unwrap();
        let examples = std::fs::read_dir(root.join("examples")).unwrap();
        let dirs: Vec<PathBuf> = examples
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .chain([root.to_owned()])
            .collect();
        for dir in dirs {
            let library = Some(dir.join("library"))
                .filter(|library| library.is_dir())
                .unwrap_or_else(|| written.path().to_owned());
            let request = |config: Option<&str>| Request {
                home: PathBuf::from("/home/sally"),
                repo_root: dir.clone(),
                library_dir: library.clone(),
                extra_files: vec![],
                config: config.map(str::to_owned),
            };
            let table = read_configs(&request(None), &files)
                .unwrap_or_else(|error| panic!("{}: {error:#}", dir.display()));
            let names = table.configs.keys().map(|key| key.name.clone());
            for name in names {
                let result = load(&request(Some(&name)), &files);

                assert!(result.is_ok(), "{} {name}: {result:?}", dir.display());
            }
        }
    }

    #[test]
    fn load__f_file_of_any_name__read_by_its_documents() {
        let files = testing::FakeFiles::with(&[("/srv/ci/other.yml", "name: ci\nimage: debian\n")]);
        let request = testing::request(Some("ci"), &["/srv/ci/other.yml"]);

        let loaded = load(&request, &files).unwrap();

        let steps: Vec<&str> = loaded
            .chain
            .iter()
            .map(|step| step.config.as_str())
            .collect();
        assert_eq!(steps, ["ci"]);
    }

    #[test]
    fn chain_files__a_file_in_several_steps__once_in_fold_order() {
        let files = [
            (
                "/home/sally/.config/viz-shell/default.vz.yml",
                "image: debian\n",
            ),
            (
                "/home/sally/repos/app/app.vz.yml",
                "extends: default\n---\nname: ci\nextends: default\n",
            ),
        ];

        let loaded = testing::load(&files, Some("ci")).unwrap();

        let paths: Vec<PathBuf> = loaded
            .chain_files()
            .into_iter()
            .map(|source| source.path)
            .collect();
        assert_eq!(
            paths,
            [
                PathBuf::from("/home/sally/.config/viz-shell/default.vz.yml"),
                PathBuf::from("/home/sally/repos/app/app.vz.yml"),
            ]
        );
    }
}
