//! Manual pages from the interface: what the user may type, and the
//! environment `man` runs with.
//!
//! `man` shows pages in `less`. Both are started from a fixed argv with an
//! environment that turns on less's secure mode (`LESSSECURE=1`: no shell
//! escape, no editor, no log file, no input preprocessor, no other file).
//! The interface starts `man` from an empty environment (`clean_env`), so
//! GROFF_*, MANPATH and the like cannot change what runs; this list then
//! sets only what is needed.

/// What was typed in the manual-page dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// One page, optionally in a section: `ls`, `5 passwd`.
    Page {
        section: Option<String>,
        name: String,
    },
    /// Several words: pages whose name or description has all of them
    /// (`apropos --and`).
    Search(Vec<String>),
}

fn valid_word(w: &str) -> bool {
    !w.is_empty()
        && w.len() <= 64
        && !w.starts_with('-')
        && w.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b':' | b'-'))
}

fn valid_section(w: &str) -> bool {
    (1..=4).contains(&w.len())
        && w.as_bytes()[0].is_ascii_digit()
        && w.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Parse the dialog's text. Words start with a letter or digit and contain
/// only letters, digits and `. _ + : -`, so nothing is read as an option.
pub fn parse(text: &str) -> Result<Request, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Err("type the name of a command, or words to search for".into());
    }
    if words.len() > 8 {
        return Err("use at most 8 words".into());
    }
    if let Some(bad) = words.iter().find(|w| !valid_word(w)) {
        return Err(format!(
            "{bad:?}: use letters, digits and . _ + : - only (no leading -)"
        ));
    }
    match words.as_slice() {
        [name] => Ok(Request::Page {
            section: None,
            name: name.to_string(),
        }),
        [section, name] if valid_section(section) => Ok(Request::Page {
            section: Some(section.to_string()),
            name: name.to_string(),
        }),
        _ => Ok(Request::Search(
            words.iter().map(|w| w.to_string()).collect(),
        )),
    }
}

impl Request {
    /// Arguments after `man` for a page, or after `apropos` for a search
    /// (an absolute path is prepended by the caller).
    pub fn args(&self) -> Vec<String> {
        match self {
            Request::Page { section, name } => {
                let mut v: Vec<String> = section.iter().cloned().collect();
                v.push(name.clone());
                v
            }
            Request::Search(words) => {
                let mut v = vec!["--and".to_string()];
                v.extend(words.iter().cloned());
                v
            }
        }
    }
}

/// Environment for `man` and its pager.
pub fn environment(less: &str) -> Vec<(String, String)> {
    [
        ("MANPAGER", less),
        ("PAGER", less),
        ("LESSSECURE", "1"),
        ("LESSOPEN", ""),
        ("LESSCLOSE", ""),
        ("LESSKEYIN", "/dev/null"),
        ("LESSEDIT", ""),
        ("VISUAL", ""),
        ("EDITOR", ""),
        ("MANOPT", ""),
        ("BROWSER", ""),
        ("HTMLPAGER", ""),
        ("MANROFFOPT", ""),
        ("LESSSECURE_ALLOW", ""),
        ("LESS", ""),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_sections_and_searches() {
        assert_eq!(
            parse(" ls ").expect("ok"),
            Request::Page {
                section: None,
                name: "ls".into()
            }
        );
        let p = parse("5 passwd").expect("ok");
        assert_eq!(p.args(), ["5", "passwd"]);
        assert_eq!(parse("systemd.unit").expect("ok").args(), ["systemd.unit"]);
        let s = parse("copy files").expect("ok");
        assert_eq!(s.args(), ["--and", "copy", "files"]);
        assert_eq!(parse("3p printf").expect("ok").args(), ["3p", "printf"]);
    }

    #[test]
    fn nothing_is_read_as_an_option_or_by_a_shell() {
        for bad in [
            "",
            "   ",
            "-H ls",
            "--html=x ls",
            "ls;id",
            "$(id)",
            "a/b",
            "`id`",
            "ls |less",
            "a b c d e f g h i",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_pager_runs_in_secure_mode() {
        let env = environment("/store/bin/less");
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("LESSSECURE"), Some("1"));
        assert_eq!(get("MANPAGER"), Some("/store/bin/less"));
        assert_eq!(get("PAGER"), Some("/store/bin/less"));
        assert_eq!(get("LESSOPEN"), Some(""));
        assert_eq!(get("MANOPT"), Some(""));
        assert_eq!(get("LESSSECURE_ALLOW"), Some(""));
    }
}
