//! A tool's life after it is written: review, promotion with the access the user saw, demotion, revoking, extending, restoring a version, re-checking and reverting.

use super::*;
use crate::chat::error::ChatError;

/// What the approval card shows for a promotion: the script, the description the model will read
/// and the capabilities asked for.
pub fn review(tool: &ScriptTool) -> Value {
    json!({
        "tool": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
        "access": if tool.requested.is_empty() {
            "none: it can only compute"
        } else {
            "the network hosts, folders and tools below"
        },
        "network": tool.requested.http_hosts,
        "reads": tool.requested.read_prefixes,
        "writes": tool.requested.write_prefixes,
        "credentials": tool.requested.credentials,
        "calls": tool.requested.tools,
        "script": tool.script,
    })
}

/// Makes an approved candidate active: its regression checks must pass, then its capabilities are
/// granted (for `days`, or until revoked) and its script pinned. Its last good test run becomes a
/// check.
/// [`promote`], only if the tool is still exactly what the user was shown: its script,
/// description, parameters and the access it asks for. Anything changed in between (another call
/// in the same step, another chat) is refused, not approved unseen.
pub async fn promote_as_shown(
    config_dir: &Path,
    shown: &ScriptTool,
    days: Option<i64>,
) -> Result<String, ChatError> {
    promote_with(config_dir, &shown.name, days, Some(shown)).await
}

pub async fn promote(
    config_dir: &Path,
    name: &str,
    days: Option<i64>,
) -> Result<String, ChatError> {
    promote_with(config_dir, name, days, None).await
}

/// Where the last state the app signed for each tool is kept: History keeps only the states a
/// save replaced, so once a tool's own file is changed outside the app, this copy is the one
/// that still says what the app last signed.
pub(super) fn signed_copy(config_dir: &Path, name: &str) -> Result<PathBuf, ChatError> {
    let file = path(config_dir, name)?;
    Ok(chat_dir(config_dir).join("script-tools-signed").join(
        file.file_name()
            .ok_or_else(|| ChatError::Invalid("bad tool name".into()))?,
    ))
}

/// The last state of a tool the app signed: its signed copy, else the newest History entry that
/// verifies.
pub(super) fn last_signed(config_dir: &Path, name: &str) -> Option<ScriptTool> {
    let copy = signed_copy(config_dir, name)
        .ok()
        .and_then(|p| crate::chat::at_rest::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str::<ScriptTool>(&text).ok())
        .filter(|t| t.name == name && verifies(config_dir, t));
    copy.or_else(|| {
        crate::chat::versions::list(config_dir, "tools", &format!("script-{name}"))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| serde_json::from_value::<ScriptTool>(v.value).ok())
            .find(|v| v.name == name && signed_by_app(config_dir, v))
    })
}

/// A last signed state's regression checks, its pending one included.
pub(super) fn signed_checks(last: &ScriptTool) -> Vec<Check> {
    let mut checks = last.checks.clone();
    if let Some(check) = last.pending_check.clone() {
        if !checks.iter().any(|c| c.args == check.args) {
            checks.push(check);
        }
    }
    checks
}

/// For a file changed outside the app, what the signature vouched for and the approval card does
/// not show (its regression checks, the versions set aside, the monitor's and History's dates) is
/// taken from the last state the app signed, so a promotion never signs what was written around
/// it. With no signed state left, the promotion is refused: the tool is written again instead,
/// and tested afresh.
pub(super) fn take_vouched(config_dir: &Path, tool: &mut ScriptTool) -> Result<(), ChatError> {
    if verifies(config_dir, tool) {
        return Ok(());
    }
    let Some(last) = last_signed(config_dir, &tool.name) else {
        // Never signed at all (written before the key was loaded, or by a build that did not
        // sign): nothing was vouched for, so nothing is taken away; it is promoted as written,
        // with no rollback or restore date of its own.
        if tool.signature.is_none() && !ever_signed(config_dir, &tool.name) {
            tool.monitor_reverted_at = None;
            tool.restored_at = None;
            return Ok(());
        }
        return Err(ChatError::Invalid(format!(
            "`{}`'s file was changed outside the app and no state the app signed remains to take its regression checks from, so it was not promoted; write it again with create_tool so it is tested afresh.",
            tool.name
        )));
    };
    tool.checks = signed_checks(&last);
    tool.pending_check = None;
    tool.set_aside = last.set_aside;
    tool.monitor_reverted_at = last.monitor_reverted_at;
    tool.restored_at = last.restored_at;
    tool.approved_at = last.approved_at;
    tool.created_at = last.created_at;
    Ok(())
}

/// Saves a change that only takes something away (demoting, revoking, retiring). A file changed
/// outside the app is saved as it is, unsigned: signing it would vouch for what was written
/// around it, and it stays unoffered until promoted, which takes those fields from the last state
/// the app signed.
pub(super) fn save_taking_away(config_dir: &Path, tool: &ScriptTool) -> Result<(), ChatError> {
    let on_disk = load(config_dir, &tool.name);
    if on_disk.as_ref().is_none_or(|t| verifies(config_dir, t)) {
        return save_resigned(config_dir, tool);
    }
    let path = path(config_dir, &tool.name)?;
    let mut tool = tool.clone();
    tool.signature = None;
    if let Some(previous) = on_disk {
        crate::chat::versions::keep(
            config_dir,
            "tools",
            &format!("script-{}", tool.name),
            &previous,
            &tool,
        )?;
    }
    crate::chat::at_rest::write(
        path,
        serde_json::to_string_pretty(&tool).map_err(ChatError::internal)?,
    )
    .map_err(ChatError::internal)
}

pub(super) async fn promote_with(
    config_dir: &Path,
    name: &str,
    days: Option<i64>,
    shown: Option<&ScriptTool>,
) -> Result<String, ChatError> {
    sandbox::cache_in(config_dir);
    #[cfg(not(test))]
    ensure_signing_key(config_dir).await;
    let mut first = load(config_dir, name)
        .ok_or_else(|| ChatError::Invalid("There is no tool by that name.".into()))?;
    take_vouched(config_dir, &mut first)?;
    let failing = failing_checks(&first.script, &first.checks).await;
    if !failing.is_empty() {
        return Err(ChatError::Invalid(format!(
            "`{name}` fails its regression checks, so it was not promoted: {}",
            failing.join("; ")
        )));
    }
    // Loaded again after the waits above, and promoted only if it is still what was checked and,
    // for a card, what the user was shown: nothing written in between is approved unseen.
    let mut tool = load(config_dir, name)
        .ok_or_else(|| ChatError::Invalid("There is no tool by that name.".into()))?;
    take_vouched(config_dir, &mut tool)?;
    let expected = shown.unwrap_or(&first);
    if approval_hash(&tool) != approval_hash(expected)
        || tool.requested != expected.requested
        || approval_hash(&tool) != approval_hash(&first)
    {
        return Err(ChatError::Invalid(format!(
            "`{name}` changed after the user was shown it, so it was not promoted; promote it again so they see what it is now."
        )));
    }
    if let Some(check) = tool.pending_check.take() {
        if !tool.checks.iter().any(|c| c.args == check.args) {
            tool.checks.push(check);
        }
    }
    tool.granted_until = days
        .filter(|d| *d > 0)
        .map(|d| Utc::now() + chrono::Duration::days(d));
    tool.granted = tool.requested.clone();
    tool.expires.clear();
    tool.status = Status::Active;
    // A version brought back from History and not approved since (a draft, say) has its own week
    // to build from this approval before it is compared week over week, not the weeks of the
    // versions it set aside. A restore already approved keeps its date and so its 14 days; any
    // other approval is compared with the version it replaces.
    let restore_unapproved = tool
        .restored_at
        .is_some_and(|at| tool.approved_at.is_none_or(|approved| at > approved));
    if restore_unapproved {
        tool.restored_at = Some(Utc::now());
    }
    tool.approved_at = Some(Utc::now());
    tool.approved_hash = Some(approval_hash(&tool));
    // An approval is judged on its own calls from now: a rollback before it holds nothing back.
    tool.monitor_reverted_at = None;
    // Approved again by the user: no longer set aside.
    let approved = approval_hash(&tool);
    tool.set_aside.retain(|h| h != &approved);
    // The user saw it on the card and approved it: signed afresh.
    save_resigned(config_dir, &tool)?;
    crate::chat::traces::register(
        config_dir,
        "script_tool",
        name,
        tool.approved_hash.as_deref(),
        None,
        Status::Active.as_str(),
    );
    Ok(format!(
        "`{name}` is now a tool, available in every chat as script__{name}."
    ))
}

/// Takes a tool off offer until it is promoted again: the lifecycle does this to one that keeps
/// failing.
pub fn demote(config_dir: &Path, name: &str) -> Result<(), ChatError> {
    let mut tool = load(config_dir, name)
        .ok_or_else(|| ChatError::Invalid("There is no tool by that name.".into()))?;
    // Judged on the file as it is, before this change.
    forget_approval_if_tampered(config_dir, &mut tool);
    tool.status = Status::Candidate;
    tool.granted = Capabilities::default();
    tool.expires.clear();
    save_taking_away(config_dir, &tool)?;
    crate::chat::traces::set_status(config_dir, "script_tool", name, Status::Candidate.as_str());
    Ok(())
}

/// Takes back a tool's capabilities; it stays on offer, able only to compute.
pub fn revoke(config_dir: &Path, name: &str) -> Result<(), ChatError> {
    let mut tool = load(config_dir, name)
        .ok_or_else(|| ChatError::Invalid("There is no tool by that name.".into()))?;
    // Judged on the file as it is, before this change: a tool the app saved stays on offer.
    forget_approval_if_tampered(config_dir, &mut tool);
    tool.granted = Capabilities::default();
    tool.requested = Capabilities::default();
    tool.granted_until = None;
    tool.expires.clear();
    save_taking_away(config_dir, &tool)
}

/// Before a path that re-signs a file (taking access away, retiring), a file changed outside the
/// app loses its approval: it is signed only as a candidate the user must approve again, never
/// as the active tool it now claims to be.
pub(super) fn forget_approval_if_tampered(config_dir: &Path, tool: &mut ScriptTool) {
    if !verifies(config_dir, tool) {
        tool.approved_hash = None;
        tool.approved_at = None;
        if tool.status == Status::Active {
            tool.status = Status::Candidate;
        }
    }
}

/// What a capability request would change, for its card: what the tool may reach now, what lapsed
/// (and is not renewed), and what it asks for.
pub fn access_card(tool: &ScriptTool, wanted: &Capabilities) -> Value {
    let active = active_grants(tool, Utc::now());
    let lapsed: Vec<String> = keyed(&tool.granted)
        .into_iter()
        .filter(|k| !keyed(&active).contains(k))
        .collect();
    json!({ "hasNow": active, "lapsed": lapsed, "asksFor": wanted })
}

/// More access for a tool, once the user approved the card built from `shown`: what it asks for,
/// for `days` or until revoked, added to what it may still reach. Each grant keeps its own
/// lapse: a short request does not shorten a lasting grant, and a lapsed grant is not renewed.
/// Refused when the tool changed after the card was shown.
pub fn extend(
    config_dir: &Path,
    shown: &ScriptTool,
    wanted: &Capabilities,
    days: Option<i64>,
) -> Result<(), ChatError> {
    let mut tool = load(config_dir, &shown.name)
        .ok_or_else(|| ChatError::Invalid("There is no tool by that name.".into()))?;
    let unchanged = approval_hash(&tool) == approval_hash(shown)
        && tool.granted == shown.granted
        && tool.requested == shown.requested
        && tool.expires == shown.expires
        && tool.granted_until == shown.granted_until;
    if !unchanged {
        return Err(ChatError::Invalid(format!(
            "`{}` changed after the user was shown the request, so nothing was granted; ask again.",
            shown.name
        )));
    }
    let now = Utc::now();
    let active = active_grants(&tool, now);
    // Lapses that came from the tool-wide date become each grant's own.
    let mut expires = std::collections::BTreeMap::new();
    for key in keyed(&active) {
        if let Some(until) = tool.expires.get(&key).copied().or(tool.granted_until) {
            expires.insert(key, until);
        }
    }
    let until = days
        .filter(|d| *d > 0)
        .map(|d| now + chrono::Duration::days(d));
    let had = keyed(&active);
    for key in keyed(wanted) {
        // Asked for again, a grant keeps whichever lasts longer: lasting beats any date.
        let lasting_already = had.contains(&key) && !expires.contains_key(&key);
        match until {
            _ if lasting_already => {}
            Some(until) => {
                let later = expires.get(&key).map_or(until, |&e| e.max(until));
                expires.insert(key, later);
            }
            None => {
                expires.remove(&key);
            }
        }
    }
    tool.granted = active.with(wanted);
    tool.requested = tool.granted.clone();
    tool.expires = expires;
    tool.granted_until = None;
    save(config_dir, &tool)
}

/// Brings back an earlier version from History. Its script comes back; its access does not grow
/// past what the tool may reach now, and a version that does not verify (written into the history
/// from outside) comes back only as a candidate, to be promoted again.
pub fn restore_version(config_dir: &Path, mut old: ScriptTool) -> Result<(), ChatError> {
    let now = Utc::now();
    let current = load(config_dir, &old.name);
    let limit = current
        .as_ref()
        .filter(|c| verifies(config_dir, c))
        .map(|c| active_grants(c, now))
        .unwrap_or_default();
    let approved = signed_by_app(config_dir, &old)
        && old.approved_hash.as_deref() == Some(approval_hash(&old).as_str());
    if !approved {
        old.status = Status::Candidate;
        old.approved_hash = None;
    }
    old.granted = old.granted.within(&limit);
    if let Some(current) = &current {
        old.expires = current.expires.clone();
        old.granted_until = current.granted_until;
    }
    // Every version approved after the one brought back, the current one included, is set
    // aside by the user's choice: the monitor never rolls forward to any of them.
    // An approved version is placed by when it was approved; a draft, or a version that does not
    // verify, by when it was written: the approvals after that are set aside, never those before.
    // A History entry changed outside the app proves nothing about when it was written: every
    // approval is set aside, and its checks are the last signed ones, never its own.
    let verified = signed_by_app(config_dir, &old);
    let restored_from = if approved {
        old.approved_at.or(Some(old.created_at))
    } else if verified {
        Some(old.created_at)
    } else {
        Some(DateTime::<Utc>::MIN_UTC)
    };
    let signed_current = current
        .as_ref()
        .filter(|c| verifies(config_dir, c))
        .cloned()
        .or_else(|| last_signed(config_dir, &old.name));
    if !verified {
        old.checks = signed_current
            .as_ref()
            .map(signed_checks)
            .unwrap_or_default();
        old.pending_check = None;
        // Its dates are not taken on its word: as far as the app knows, it was written now.
        old.created_at = now;
        old.approved_at = None;
    }
    let restored = approval_hash(&old);
    // What the user set aside, as the app last signed it: a current file changed outside the app
    // drops nothing from it.
    let mut set_aside = signed_current
        .as_ref()
        .map(|c| c.set_aside.clone())
        .unwrap_or_default();
    let mut later: Vec<String> =
        crate::chat::versions::list(config_dir, "tools", &format!("script-{}", old.name))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| serde_json::from_value::<ScriptTool>(v.value).ok())
            .filter(|v| {
                v.status == Status::Active
                    && restored_from.is_some_and(|from| v.approved_at.is_some_and(|at| at > from))
            })
            .map(|v| approval_hash(&v))
            .collect();
    if let Some(current) = signed_current
        .as_ref()
        .filter(|c| c.status == Status::Active)
    {
        later.push(approval_hash(current));
    }
    for hash in later {
        if hash != restored && !set_aside.contains(&hash) {
            set_aside.push(hash);
        }
    }
    // Brought back, it is no longer set aside itself.
    set_aside.retain(|h| h != &restored);
    old.set_aside = set_aside;
    // Back in use from now, by the user's choice: judged on its own calls from here, never on
    // the calls of the versions it set aside. A draft brought back carries the mark into its
    // approval.
    if old.status == Status::Active {
        old.approved_at = Some(now);
    }
    old.restored_at = Some(now);
    old.monitor_reverted_at = None;
    save_resigned(config_dir, &old)?;
    crate::chat::traces::register(
        config_dir,
        "script_tool",
        &old.name,
        old.approved_hash.as_deref(),
        None,
        old.status.as_str(),
    );
    Ok(())
}

/// What the daily regression checks did.
#[derive(Debug, Default, PartialEq)]
pub struct Checked {
    /// Brought back to their last approved version that still passes.
    pub reverted: Vec<String>,
    /// Sent back to candidate: no earlier version passes either.
    pub demoted: Vec<String>,
}

/// Runs every active tool's regression checks. A tool that fails one goes back to its previous
/// approved version when that version passes them; otherwise it goes back to candidate.
pub async fn check_all(config_dir: &Path) -> Checked {
    sandbox::cache_in(config_dir);
    let mut done = Checked::default();
    for tool in list(config_dir)
        .into_iter()
        .filter(|t| t.status == Status::Active && !t.checks.is_empty())
    {
        if failing_checks(&tool.script, &tool.checks).await.is_empty() {
            continue;
        }
        if matches!(revert(config_dir, &tool.name, &tool.checks).await, Ok(true)) {
            done.reverted.push(tool.name);
        } else if demote(config_dir, &tool.name).is_ok() {
            done.demoted.push(tool.name);
        }
    }
    done
}

/// Brings a tool back to its newest earlier version that the user approved, runs a different
/// script, and passes `checks`. The version it replaces is kept, so this too can be undone.
/// `Ok(false)` when there is no such version.
pub async fn revert(config_dir: &Path, name: &str, checks: &[Check]) -> Result<bool, ChatError> {
    let current = load(config_dir, name)
        .ok_or_else(|| ChatError::Invalid("There is no tool by that name.".into()))?;
    for version in crate::chat::versions::list(config_dir, "tools", &format!("script-{name}"))? {
        let Ok(old) = serde_json::from_value::<ScriptTool>(version.value) else {
            continue;
        };
        // Only a version the app saved as approved: its hash pins its script, and its
        // signature shows nobody wrote it into the history from outside.
        let approved = old.status == Status::Active
            && old.approved_hash.as_deref() == Some(approval_hash(&old).as_str())
            && signed_by_app(config_dir, &old);
        // Never forward to a version the user set aside by restoring an earlier one.
        if !approved
            || old.script == current.script
            || current.set_aside.contains(&approval_hash(&old))
        {
            continue;
        }
        if failing_checks(&old.script, checks).await.is_empty() {
            // Checking took a while: a revoke or any other change meanwhile wins, and the
            // rollback waits for the next pass.
            let latest = load(config_dir, name)
                .ok_or_else(|| ChatError::Invalid("There is no tool by that name.".into()))?;
            if latest.granted != current.granted
                || latest.requested != current.requested
                || latest.expires != current.expires
                || latest.status != current.status
                || approval_hash(&latest) != approval_hash(&current)
            {
                return Ok(false);
            }
            // The script goes back; the access does not grow: it keeps only what the tool may
            // reach now, so a revoke or a lapse is never undone by a rollback.
            let mut back = old;
            back.granted = back.granted.within(&active_grants(&current, Utc::now()));
            back.requested = back.requested.within(&current.requested);
            back.expires = current.expires.clone();
            back.granted_until = current.granted_until;
            // Back in use from now: judged on its calls from here, and not rolled back again
            // within the week on the failures of the version it replaced.
            back.approved_at = Some(Utc::now());
            back.monitor_reverted_at = Some(Utc::now());
            back.set_aside = current.set_aside.clone();
            back.restored_at = None;
            save(config_dir, &back)?;
            crate::chat::traces::register(
                config_dir,
                "script_tool",
                name,
                back.approved_hash.as_deref(),
                None,
                back.status.as_str(),
            );
            return Ok(true);
        }
    }
    Ok(false)
}
