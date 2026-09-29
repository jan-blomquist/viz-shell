//! `banner: true`: the viz-shell banner above an interactive shell, with
//! what the shell may do.

const ART: &str = include_str!("../templates/banner.txt");

/// What the banner reports.
pub struct Status<'a> {
    pub repo: &'a str,
    pub profile: Option<&'a str>,
    pub sudo: bool,
    pub docker: bool,
    pub host_network: bool,
}

pub fn render(status: &Status) -> String {
    let yes_no = |on: bool| if on { "yes" } else { "no" };
    format!(
        "{ART}\n  {} | profile: {} | sudo: {} | docker: {} | host network: {}\n\n",
        status.repo,
        status.profile.unwrap_or("none"),
        yes_no(status.sudo),
        yes_no(status.docker),
        yes_no(status.host_network),
    )
}

#[cfg(test)]
#[allow(non_snake_case)] // unit__scenario__expected test names
mod tests {
    use super::*;

    #[test]
    fn render__status__art_then_one_status_line() {
        let status = Status {
            repo: "app",
            profile: Some("trusted"),
            sudo: true,
            docker: true,
            host_network: false,
        };

        let banner = render(&status);

        assert!(banner.starts_with(ART), "{banner}");
        assert!(
            banner.ends_with(
                "\n  app | profile: trusted | sudo: yes | docker: yes | host network: no\n\n"
            ),
            "{banner}"
        );
    }

    #[test]
    fn render__no_profile__says_none() {
        let status = Status {
            repo: "app",
            profile: None,
            sudo: false,
            docker: false,
            host_network: false,
        };

        assert!(render(&status).contains("| profile: none |"));
    }
}
