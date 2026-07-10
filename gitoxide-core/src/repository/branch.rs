use crate::OutputFormat;

pub mod list {
    pub enum Kind {
        Local,
        All,
    }

    pub struct Options {
        pub kind: Kind,
        pub prefix: Option<String>,
        #[cfg(feature = "reference-fuzzy-nucleo")]
        pub fuzzy: Option<String>,
        pub sort: bool,
    }
}

pub fn list(
    repo: gix::Repository,
    out: &mut dyn std::io::Write,
    format: OutputFormat,
    options: list::Options,
) -> anyhow::Result<()> {
    if format != OutputFormat::Human {
        anyhow::bail!("JSON output isn't supported");
    }

    let platform = repo.references()?;

    let (show_local, show_remotes) = match options.kind {
        list::Kind::Local => (true, false),
        list::Kind::All => (true, true),
    };
    let prefix = options.prefix.as_deref();

    #[cfg(feature = "reference-fuzzy-nucleo")]
    if let Some(query) = options.fuzzy.as_deref() {
        let max_results = 128;
        let mut hits = if let Some(prefix) = prefix {
            repo.find_references_fuzzy_prefixed(query, max_results, prefix)?
        } else {
            repo.find_references_fuzzy(query, max_results)?
        };
        if options.sort {
            hits.sort_by(|lhs, rhs| lhs.name().as_bstr().cmp(rhs.name().as_bstr()));
        }
        for hit in hits {
            let name = hit.name().as_bstr();
            let is_local = name.starts_with(b"refs/heads/");
            let is_remote = name.starts_with(b"refs/remotes/");

            if !(is_local && show_local || is_remote && show_remotes) {
                continue;
            }

            writeln!(out, "{}\t{}", hit.name().shorten(), hit.score())?;
        }
        return Ok(());
    }

    if let Some(prefix) = prefix {
        let mut local_branch_names = Vec::new();
        let mut remote_branch_names = Vec::new();

        for branch in platform.prefixed(prefix)?.flatten() {
            let full_name = branch.name().as_bstr();
            if show_local && full_name.starts_with(b"refs/heads/") {
                local_branch_names.push(branch.name().shorten().to_string());
            } else if show_remotes && full_name.starts_with(b"refs/remotes/") {
                remote_branch_names.push(branch.name().shorten().to_string());
            }
        }

        if options.sort {
            local_branch_names.sort();
            remote_branch_names.sort();
        }

        for branch_name in local_branch_names {
            writeln!(out, "{branch_name}")?;
        }
        for branch_name in remote_branch_names {
            writeln!(out, "{branch_name}")?;
        }

        return Ok(());
    }

    if show_local {
        let mut branch_names: Vec<String> = platform
            .local_branches()?
            .flatten()
            .map(|branch| branch.name().shorten().to_string())
            .collect();

        if options.sort {
            branch_names.sort();
        }

        for branch_name in branch_names {
            writeln!(out, "{branch_name}")?;
        }
    }

    if show_remotes {
        let mut branch_names: Vec<String> = platform
            .remote_branches()?
            .flatten()
            .map(|branch| branch.name().shorten().to_string())
            .collect();

        if options.sort {
            branch_names.sort();
        }

        for branch_name in branch_names {
            writeln!(out, "{branch_name}")?;
        }
    }

    Ok(())
}
