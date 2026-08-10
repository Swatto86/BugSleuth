//! Proving attribution-bearing commits have not escaped through Git refs.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::cancel::Cancel;

use super::super::{network, observed::git};

pub(super) async fn refuse(
    repo: &Path,
    ids: &[String],
    cancel: &Cancel,
    timeout: Duration,
) -> anyhow::Result<()> {
    for id in ids {
        let refs = git(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "--contains",
                id,
                "refs/remotes/",
                "refs/tags/",
            ],
        )?;
        if !refs.trim().is_empty() {
            anyhow::bail!(
                "commit {} is already reachable through a remote-tracking branch or local tag, so its message cannot be rewritten without forking that history",
                &id[..id.len().min(8)]
            );
        }
    }

    for target in remote_tag_targets(repo, cancel, timeout).await? {
        for id in ids {
            if remote_tag_contains(repo, id, &target)? {
                anyhow::bail!(
                    "commit {} is already reachable through a tag on a configured remote, so its message cannot be rewritten without forking published history",
                    &id[..id.len().min(8)]
                );
            }
        }
    }
    Ok(())
}

async fn remote_tag_targets(
    repo: &Path,
    cancel: &Cancel,
    timeout: Duration,
) -> anyhow::Result<Vec<String>> {
    let remotes = git(repo, &["remote"])?;
    let mut targets = Vec::new();
    for remote in remotes.lines().filter(|name| !name.is_empty()) {
        for url in remote_urls(repo, remote)? {
            let listing = network::git(repo, &["ls-remote", "--tags", "--", &url], cancel, timeout)
                .await
                .map_err(|error| {
                    anyhow::anyhow!("could not inspect tags on remote {remote}: {error}")
                })?;
            targets.extend(parse_remote_tags(&listing)?);
        }
    }
    targets.sort();
    targets.dedup();
    Ok(targets)
}

fn remote_urls(repo: &Path, remote: &str) -> anyhow::Result<Vec<String>> {
    let mut urls = Vec::new();
    for args in [
        &["remote", "get-url", "--all", "--", remote][..],
        &["remote", "get-url", "--push", "--all", "--", remote][..],
    ] {
        urls.extend(
            git(repo, args)?
                .lines()
                .filter(|url| !url.is_empty())
                .map(str::to_string),
        );
    }
    urls.sort();
    urls.dedup();
    if urls.is_empty() {
        anyhow::bail!("configured remote {remote} has no readable URL");
    }
    Ok(urls)
}

fn parse_remote_tags(listing: &str) -> anyhow::Result<Vec<String>> {
    let mut tags = BTreeMap::<String, (String, bool)>::new();
    for line in listing.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let oid = fields.next().unwrap_or_default();
        let reference = fields.next().unwrap_or_default();
        if fields.next().is_some()
            || !matches!(oid.len(), 40 | 64)
            || !oid.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            anyhow::bail!("a remote returned a malformed tag listing");
        }
        let Some(tag) = reference.strip_prefix("refs/tags/") else {
            anyhow::bail!("a remote returned a non-tag in its tag listing");
        };
        let (tag, peeled) = tag
            .strip_suffix("^{}")
            .map_or((tag, false), |tag| (tag, true));
        if tag.is_empty() {
            anyhow::bail!("a remote returned an empty tag name");
        }
        match tags.get_mut(tag) {
            Some((existing, existing_peeled)) if peeled && !*existing_peeled => {
                *existing = oid.to_string();
                *existing_peeled = true;
            }
            Some((existing, existing_peeled)) if *existing_peeled == peeled => {
                if existing != oid {
                    anyhow::bail!("a remote returned conflicting objects for one tag");
                }
            }
            Some(_) => {}
            None => {
                tags.insert(tag.to_string(), (oid.to_string(), peeled));
            }
        }
    }
    Ok(tags.into_values().map(|(oid, _)| oid).collect())
}

fn remote_tag_contains(repo: &Path, id: &str, target: &str) -> anyhow::Result<bool> {
    let peeled = git(repo, &["rev-parse", "--verify", &format!("{target}^{{}}")])?
        .trim()
        .to_string();
    match git(repo, &["cat-file", "-t", &peeled])?.trim() {
        "tree" | "blob" => return Ok(false),
        "commit" => {}
        kind => anyhow::bail!("a remote tag peeled to unsupported object type {kind}"),
    }
    if id == peeled {
        return Ok(true);
    }
    let range = format!("{id}..{peeled}");
    Ok(!git(
        repo,
        &["rev-list", "--ancestry-path", "--max-count=1", &range],
    )?
    .trim()
    .is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_non_commit_targets_are_irrelevant_but_missing_targets_fail_closed() {
        let dir = std::env::temp_dir().join(format!("bugsleuth-tag-kind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create repository");
        git(&dir, &["init", "-q"]).expect("init");
        git(&dir, &["config", "user.email", "t@example.com"]).expect("email");
        git(&dir, &["config", "user.name", "Tester"]).expect("name");
        std::fs::write(dir.join("a.txt"), "one").expect("write");
        git(&dir, &["add", "-A"]).expect("add");
        git(&dir, &["commit", "-qm", "one"]).expect("commit");
        let head = git(&dir, &["rev-parse", "HEAD"])
            .expect("head")
            .trim()
            .to_string();
        let tree = git(&dir, &["rev-parse", "HEAD^{tree}"])
            .expect("tree")
            .trim()
            .to_string();
        assert!(!remote_tag_contains(&dir, &head, &tree).expect("known tree"));
        assert!(remote_tag_contains(&dir, &head, &"0".repeat(40)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
