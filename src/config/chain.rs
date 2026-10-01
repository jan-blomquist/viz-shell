//! The chain, rendered three ways from one `Vec<Step>`: the header lines of
//! `--show-effective-config`, the `vz.chain` label a container carries, and
//! the banner's `Chain:` line. Invariant: the banner reads the chain back from
//! label entries, so a container shows the same line when attached as when
//! created.

use std::path::{Component, Path, PathBuf};

use super::configs::Step;
use super::paths::{shown, tilde};
use super::scan::Scope;

/// A step as the `vz.chain` label records it: `<config>@<file>`. A
/// repository file is relative to the repository root, `..` included; a
/// library file is `~/…` or absolute. So the text alone says which it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub config: String,
    pub file: String,
}

/// What a label shows where entries were cut to fit.
pub const CUT: &str = "…";

impl Link {
    /// An entry of the `vz.chain` label; `…` stands for entries cut.
    pub fn parse(entry: &str) -> Self {
        let (config, file) = entry.split_once('@').unwrap_or((entry, ""));
        Self {
            config: config.to_owned(),
            file: file.to_owned(),
        }
    }

    /// The label entry: `<config>@<file>`.
    pub fn entry(&self) -> String {
        format!("{}@{}", self.config, self.file)
    }

    fn is_library(&self) -> bool {
        self.file.starts_with('~') || self.file.starts_with('/')
    }

    /// The configuration, with `(library)` for a library one.
    fn banner_label(&self) -> String {
        match (self.config.as_str(), self.is_library()) {
            (CUT, _) => CUT.to_owned(),
            (config, true) => format!("{config} (library)"),
            (config, false) => config.to_owned(),
        }
    }
}

/// One link per step, in fold order.
pub fn links(chain: &[Step], repo_root: &Path, home: &Path) -> Vec<Link> {
    chain
        .iter()
        .map(|step| Link {
            config: step.config.clone(),
            file: match step.file.scope {
                Scope::Repository => relative(&step.file.path, repo_root).display().to_string(),
                Scope::Library => tilde(&step.file.path, home),
            },
        })
        .collect()
}

/// The banner's `Chain:` value: `default (library) → trusted`.
pub fn banner_line(links: &[Link]) -> String {
    let labels: Vec<String> = links.iter().map(Link::banner_label).collect();
    labels.join(" → ")
}

/// The lines above the YAML of `--show-effective-config`: one per step in
/// fold order, `# <config> (<file>)`, then `# image: <line>` for each of
/// `image_lines`.
pub fn header(
    chain: &[Step],
    image_lines: &[String],
    repo_root: &Path,
    home: &Path,
) -> Vec<String> {
    let steps = chain.iter().map(|step| {
        let file = shown(&step.file.path, repo_root, home);
        format!("# {} ({file})", step.config)
    });
    let images = image_lines.iter().map(|line| format!("# image: {line}"));
    steps.chain(images).collect()
}

/// `path` relative to `base`, with `..` for each folder of `base` it lies
/// outside of. Both absolute.
fn relative(path: &Path, base: &Path) -> PathBuf {
    let shared = path
        .components()
        .zip(base.components())
        .take_while(|(a, b)| a == b)
        .count();
    let up = base.components().count() - shared;
    std::iter::repeat_n(Component::ParentDir, up)
        .chain(path.components().skip(shared))
        .collect()
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;
    use crate::config::scan::Source;
    use crate::config::testing::{HOME, REPO};

    fn step(config: &str, path: &str, scope: Scope) -> Step {
        Step {
            config: config.to_owned(),
            file: Source {
                path: PathBuf::from(path),
                scope,
            },
        }
    }

    /// A step of `config` in the library's file at `path`.
    fn library_step(config: &str, path: &str) -> Step {
        step(config, path, Scope::Library)
    }

    /// A step of `config` in the repository's file at `path`.
    fn repository_step(config: &str, path: &str) -> Step {
        step(config, path, Scope::Repository)
    }

    const LIBRARY_BASE: &str = "/home/sally/.config/viz-shell/vz-debian-trixie.vz.yml";
    const LIBRARY_TRUSTED: &str = "/home/sally/.config/viz-shell/trusted.vz.yml";
    const APP: &str = "/home/sally/repos/app/app.vz.yml";
    const SALLY: &str = "/home/sally/repos/app/sally.vz.yml";

    fn one_step() -> Vec<Step> {
        vec![repository_step("default", APP)]
    }

    /// Two library files, then two of the repository.
    fn four_steps() -> Vec<Step> {
        vec![
            library_step("vz-debian-trixie", LIBRARY_BASE),
            library_step("trusted", LIBRARY_TRUSTED),
            repository_step("trusted", APP),
            repository_step("sally", SALLY),
        ]
    }

    fn library_only() -> Vec<Step> {
        vec![library_step("vz-debian-trixie", LIBRARY_BASE)]
    }

    /// A library folder outside the home, listed under `scan:`.
    fn library_outside_the_home() -> Vec<Step> {
        vec![library_step("team", "/srv/team/team.vz.yml")]
    }

    fn render_links(chain: &[Step]) -> Vec<Link> {
        links(chain, Path::new(REPO), Path::new(HOME))
    }

    #[test]
    fn header__each_chain__one_line_per_step_then_per_image() {
        let images = ["debian (default, app.vz.yml)".to_owned()];
        let cases: [(&str, Vec<Step>, &[&str]); 3] = [
            (
                "one step",
                one_step(),
                &[
                    "# default (app.vz.yml)",
                    "# image: debian (default, app.vz.yml)",
                ],
            ),
            (
                "four steps, two files of the repository",
                four_steps(),
                &[
                    "# vz-debian-trixie (~/.config/viz-shell/vz-debian-trixie.vz.yml)",
                    "# trusted (~/.config/viz-shell/trusted.vz.yml)",
                    "# trusted (app.vz.yml)",
                    "# sally (sally.vz.yml)",
                    "# image: debian (default, app.vz.yml)",
                ],
            ),
            (
                "the library only",
                library_only(),
                &[
                    "# vz-debian-trixie (~/.config/viz-shell/vz-debian-trixie.vz.yml)",
                    "# image: debian (default, app.vz.yml)",
                ],
            ),
        ];
        for (case, chain, expected) in cases {
            let lines = header(&chain, &images, Path::new(REPO), Path::new(HOME));

            assert_eq!(lines, expected, "{case}");
        }
    }

    #[test]
    fn entry__each_chain__config_at_file_one_per_step() {
        let cases: [(&str, Vec<Step>, &[&str]); 4] = [
            ("one step", one_step(), &["default@app.vz.yml"]),
            (
                "four steps, two files of the repository",
                four_steps(),
                &[
                    "vz-debian-trixie@~/.config/viz-shell/vz-debian-trixie.vz.yml",
                    "trusted@~/.config/viz-shell/trusted.vz.yml",
                    "trusted@app.vz.yml",
                    "sally@sally.vz.yml",
                ],
            ),
            (
                "the library only",
                library_only(),
                &["vz-debian-trixie@~/.config/viz-shell/vz-debian-trixie.vz.yml"],
            ),
            (
                "a library file outside the home",
                library_outside_the_home(),
                &["team@/srv/team/team.vz.yml"],
            ),
        ];
        for (case, chain, expected) in cases {
            let entries: Vec<String> = render_links(&chain).iter().map(Link::entry).collect();

            assert_eq!(entries, expected, "{case}");
        }
    }

    #[test]
    fn banner_line__each_chain__library_configurations_marked() {
        let cases = [
            ("one step", one_step(), "default"),
            (
                "four steps, two files of the repository",
                four_steps(),
                "vz-debian-trixie (library) → trusted (library) → trusted → sally",
            ),
            (
                "the library only",
                library_only(),
                "vz-debian-trixie (library)",
            ),
            (
                "a library file outside the home",
                library_outside_the_home(),
                "team (library)",
            ),
        ];
        for (case, chain, expected) in cases {
            let line = banner_line(&render_links(&chain));

            assert_eq!(line, expected, "{case}");
        }
    }

    /// The invariant itself: a container shows the same line attached as
    /// created, so the expectation is the line rendered from the steps.
    #[test]
    fn banner_line__read_back_from_label_entries__the_same_line() {
        let cases = [
            ("one step", one_step()),
            ("four steps, two files of the repository", four_steps()),
            ("the library only", library_only()),
            (
                "a library file outside the home",
                library_outside_the_home(),
            ),
        ];
        for (case, chain) in cases {
            let links = render_links(&chain);
            let entries: Vec<String> = links.iter().map(Link::entry).collect();

            let read_back: Vec<Link> = entries.iter().map(|entry| Link::parse(entry)).collect();

            assert_eq!(banner_line(&read_back), banner_line(&links), "{case}");
        }
    }

    #[test]
    fn banner_line__entries_cut__shows_where() {
        let links = ["default@app.vz.yml", "…", "ci@ci.vz.yml"].map(Link::parse);

        let line = banner_line(&links);

        assert_eq!(line, "default → … → ci");
    }

    #[test]
    fn entry__repository_file_outside_the_root__relative_with_dots() {
        let chain = [repository_step(
            "default",
            "/home/sally/repos/vz/examples/local/sally.vz.yml",
        )];

        let entry = render_links(&chain)[0].entry();

        assert_eq!(entry, "default@../vz/examples/local/sally.vz.yml");
    }
}
