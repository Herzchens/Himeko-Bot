use crate::commands::reload::create_engine;
use crate::config::Config;
use crate::text::normalizer::Normalizer;
use crate::{Data, RuntimeSnapshot};
use atomic_write_file::AtomicWriteFile;
use poise::CreateReply;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

pub(crate) type Error = Box<dyn std::error::Error + Send + Sync>;
pub(crate) type Context<'a> = poise::Context<'a, Data, Error>;

const CONFIG_PATH: &str = "config.yml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AllowedUserMutation {
    Added,
    AlreadyPresent,
    Removed,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AbbreviationMutation {
    Added,
    Updated { previous: String },
    Unchanged,
    Removed { previous: String },
    NotFound,
}

pub(crate) async fn require_owner(ctx: Context<'_>) -> Result<bool, Error> {
    let config = ctx.data().config_snapshot().await;
    if ctx.author().id.get() == config.permissions.owner_id {
        return Ok(true);
    }

    ctx.send(
        CreateReply::default()
            .content("❌ Chỉ bot owner mới có quyền dùng lệnh này.")
            .ephemeral(true),
    )
    .await?;
    Ok(false)
}

pub(crate) async fn add_allowed_user(
    data: &Data,
    actor_id: u64,
    user_id: u64,
) -> anyhow::Result<AllowedUserMutation> {
    mutate_allowed_user(data, actor_id, user_id, true).await
}

pub(crate) async fn remove_allowed_user(
    data: &Data,
    actor_id: u64,
    user_id: u64,
) -> anyhow::Result<AllowedUserMutation> {
    mutate_allowed_user(data, actor_id, user_id, false).await
}

async fn mutate_allowed_user(
    data: &Data,
    actor_id: u64,
    user_id: u64,
    add: bool,
) -> anyhow::Result<AllowedUserMutation> {
    if user_id == 0 {
        anyhow::bail!("Discord user ID must be non-zero");
    }

    let _transaction = data.runtime.begin_reload().await;
    let current = data.runtime_snapshot().await;
    ensure_current_owner(actor_id, current.as_ref())?;

    let raw = read_config_raw()?;
    let disk = Config::parse(&raw)?;
    let disk_has = disk.permissions.allowed_users.contains(&user_id);
    let runtime_has = current.config.permissions.allowed_users.contains(&user_id);

    if add && disk_has && runtime_has {
        return Ok(AllowedUserMutation::AlreadyPresent);
    }
    if !add && !disk_has && !runtime_has {
        return Ok(AllowedUserMutation::NotFound);
    }

    let mut users = disk.permissions.allowed_users.clone();
    let mut write_needed = false;
    let result = if add {
        if !disk_has {
            users.push(user_id);
            write_needed = true;
            AllowedUserMutation::Added
        } else {
            AllowedUserMutation::AlreadyPresent
        }
    } else {
        if disk_has {
            users.retain(|candidate| *candidate != user_id);
            write_needed = true;
        }
        AllowedUserMutation::Removed
    };

    let rendered = if write_needed {
        render_allowed_users(&raw, &users)?
    } else {
        raw
    };
    let candidate = Config::parse(&rendered)?;
    apply_candidate(data, current, candidate, &rendered, write_needed).await?;

    tracing::info!(
        actor_id,
        user_id,
        add,
        persisted = write_needed,
        "owner updated allowed_users"
    );
    Ok(result)
}

pub(crate) async fn add_abbreviation(
    data: &Data,
    actor_id: u64,
    key: &str,
    value: &str,
) -> anyhow::Result<AbbreviationMutation> {
    let _transaction = data.runtime.begin_reload().await;
    let current = data.runtime_snapshot().await;
    ensure_current_owner(actor_id, current.as_ref())?;

    let raw = read_config_raw()?;
    let disk = Config::parse(&raw)?;
    let canonical = key.to_lowercase();

    let disk_matches: Vec<(&String, &String)> = disk
        .abbreviations
        .iter()
        .filter(|(candidate, _)| candidate.to_lowercase() == canonical)
        .collect();
    let runtime_matches: Vec<(&String, &String)> = current
        .config
        .abbreviations
        .iter()
        .filter(|(candidate, _)| candidate.to_lowercase() == canonical)
        .collect();

    let disk_is_canonical = disk_matches.len() == 1
        && disk_matches[0].0 == &canonical
        && disk_matches[0].1.as_str() == value;
    let runtime_is_current = runtime_matches.len() == 1 && runtime_matches[0].1.as_str() == value;

    if disk_is_canonical && runtime_is_current {
        return Ok(AbbreviationMutation::Unchanged);
    }

    let previous = disk_matches
        .first()
        .map(|(_, previous)| (*previous).clone())
        .or_else(|| {
            runtime_matches
                .first()
                .map(|(_, previous)| (*previous).clone())
        });

    let write_needed = !disk_is_canonical;
    let rendered = if write_needed {
        upsert_abbreviation(&raw, &canonical, value)?
    } else {
        raw
    };
    let candidate = Config::parse(&rendered)?;
    apply_candidate(data, current, candidate, &rendered, write_needed).await?;

    let result = match previous {
        Some(previous) if previous != value => AbbreviationMutation::Updated { previous },
        Some(_) => AbbreviationMutation::Unchanged,
        None => AbbreviationMutation::Added,
    };

    tracing::info!(
        actor_id,
        abbreviation = %canonical,
        persisted = write_needed,
        "owner added or updated abbreviation"
    );
    Ok(result)
}

pub(crate) async fn remove_abbreviation(
    data: &Data,
    actor_id: u64,
    key: &str,
) -> anyhow::Result<AbbreviationMutation> {
    let _transaction = data.runtime.begin_reload().await;
    let current = data.runtime_snapshot().await;
    ensure_current_owner(actor_id, current.as_ref())?;

    let raw = read_config_raw()?;
    let disk = Config::parse(&raw)?;
    let canonical = key.to_lowercase();

    let disk_previous = disk
        .abbreviations
        .iter()
        .find(|(candidate, _)| candidate.to_lowercase() == canonical)
        .map(|(_, value)| value.clone());
    let runtime_previous = current
        .config
        .abbreviations
        .iter()
        .find(|(candidate, _)| candidate.to_lowercase() == canonical)
        .map(|(_, value)| value.clone());

    if disk_previous.is_none() && runtime_previous.is_none() {
        return Ok(AbbreviationMutation::NotFound);
    }

    let write_needed = disk_previous.is_some();
    let rendered = if write_needed {
        delete_abbreviation(&raw, &canonical)?
    } else {
        raw
    };
    let candidate = Config::parse(&rendered)?;
    apply_candidate(data, current, candidate, &rendered, write_needed).await?;

    let previous = disk_previous
        .or(runtime_previous)
        .expect("presence checked above");

    tracing::info!(
        actor_id,
        abbreviation = %canonical,
        persisted = write_needed,
        "owner removed abbreviation"
    );
    Ok(AbbreviationMutation::Removed { previous })
}

fn ensure_current_owner(actor_id: u64, current: &RuntimeSnapshot) -> anyhow::Result<()> {
    if actor_id != current.config.permissions.owner_id {
        anyhow::bail!("bot owner changed while the command was running; retry the command");
    }
    Ok(())
}

async fn apply_candidate(
    data: &Data,
    current: Arc<RuntimeSnapshot>,
    candidate: Config,
    rendered: &str,
    write_needed: bool,
) -> anyhow::Result<()> {
    candidate.validate_hot_reload_from(current.config.as_ref())?;

    let tts_engine = if candidate.tts.eq(&current.config.tts) {
        Arc::clone(&current.tts_engine)
    } else {
        create_engine(&candidate)
            .await
            .map_err(|error| anyhow::anyhow!(error))?
    };

    let normalizer = Arc::new(Normalizer::from_config(&candidate.abbreviations));
    let default_female = candidate.tts.default_gender == "female";
    let next = Arc::new(RuntimeSnapshot {
        config: Arc::new(candidate),
        normalizer,
        tts_engine,
        default_female,
    });

    if write_needed {
        atomic_write_config(rendered)?;
    }

    data.publish_runtime(next).await;
    data.state.set_default_female(default_female);
    Ok(())
}

fn read_config_raw() -> anyhow::Result<String> {
    fs::read_to_string(CONFIG_PATH)
        .map_err(|error| anyhow::anyhow!("failed to read {CONFIG_PATH}: {error}"))
}

fn atomic_write_config(content: &str) -> anyhow::Result<()> {
    let path = Path::new(CONFIG_PATH);
    let mut file = AtomicWriteFile::options().open(path)?;
    file.write_all(content.as_bytes())?;
    file.commit()?;
    Ok(())
}

fn render_allowed_users(raw: &str, users: &[u64]) -> anyhow::Result<String> {
    let trailing_newline = raw.ends_with('\n');
    let mut lines: Vec<String> = raw.lines().map(str::to_owned).collect();
    let (section_start, section_end) = section_bounds(&lines, "permissions")
        .ok_or_else(|| anyhow::anyhow!("config is missing the permissions section"))?;
    let child_indent = direct_child_indent(&lines, section_start, section_end);

    let key_index = (section_start + 1..section_end).find(|index| {
        leading_spaces(&lines[*index]) == child_indent
            && mapping_key_prefix(&lines[*index], "allowed_users")
    });

    let rendered_users = users
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ");

    match key_index {
        Some(index) => {
            let comment = inline_comment_suffix(&lines[index]);
            lines[index] = format!(
                "{}allowed_users: [{}]{}",
                " ".repeat(child_indent),
                rendered_users,
                comment
            );

            let nested_index = index + 1;
            while nested_index < lines.len() {
                let line = &lines[nested_index];
                if line.trim().is_empty() || leading_spaces(line) <= child_indent {
                    break;
                }
                lines.remove(nested_index);
            }
        }
        None => {
            let insertion = trailing_section_insertion(&lines, section_start, section_end);
            lines.insert(
                insertion,
                format!(
                    "{}allowed_users: [{}]",
                    " ".repeat(child_indent),
                    rendered_users
                ),
            );
        }
    }

    Ok(join_lines(lines, trailing_newline))
}

fn upsert_abbreviation(raw: &str, key: &str, value: &str) -> anyhow::Result<String> {
    let trailing_newline = raw.ends_with('\n');
    let mut lines: Vec<String> = raw.lines().map(str::to_owned).collect();

    let Some((section_start, section_end)) = section_bounds(&lines, "abbreviations") else {
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push("abbreviations:".to_string());
        lines.push(render_mapping_line(2, key, value)?);
        return Ok(join_lines(lines, trailing_newline || !raw.is_empty()));
    };

    let child_indent = direct_child_indent(&lines, section_start, section_end);
    let canonical = key.to_lowercase();
    let matches: Vec<usize> = (section_start + 1..section_end)
        .filter(|index| leading_spaces(&lines[*index]) == child_indent)
        .filter(|index| {
            parsed_mapping_key(&lines[*index])
                .is_some_and(|candidate| candidate.to_lowercase() == canonical)
        })
        .collect();

    let replacement = render_mapping_line(child_indent, &canonical, value)?;
    if let Some(first) = matches.first().copied() {
        lines[first] = replacement;
        for index in matches.iter().copied().skip(1).rev() {
            lines.remove(index);
        }
    } else {
        let insertion = trailing_section_insertion(&lines, section_start, section_end);
        lines.insert(insertion, replacement);
    }

    Ok(join_lines(lines, trailing_newline))
}

fn delete_abbreviation(raw: &str, key: &str) -> anyhow::Result<String> {
    let trailing_newline = raw.ends_with('\n');
    let mut lines: Vec<String> = raw.lines().map(str::to_owned).collect();
    let Some((section_start, section_end)) = section_bounds(&lines, "abbreviations") else {
        return Ok(raw.to_string());
    };
    let child_indent = direct_child_indent(&lines, section_start, section_end);
    let canonical = key.to_lowercase();

    let matches: Vec<usize> = (section_start + 1..section_end)
        .filter(|index| leading_spaces(&lines[*index]) == child_indent)
        .filter(|index| {
            parsed_mapping_key(&lines[*index])
                .is_some_and(|candidate| candidate.to_lowercase() == canonical)
        })
        .collect();

    for index in matches.into_iter().rev() {
        lines.remove(index);
    }
    Ok(join_lines(lines, trailing_newline))
}

fn section_bounds(lines: &[String], section: &str) -> Option<(usize, usize)> {
    let header = format!("{section}:");
    let start = lines.iter().position(|line| {
        leading_spaces(line) == 0 && significant_content(line).trim_end() == header
    })?;
    let end = (start + 1..lines.len())
        .find(|index| {
            let line = &lines[*index];
            !line.trim().is_empty()
                && !line.trim_start().starts_with('#')
                && leading_spaces(line) == 0
        })
        .unwrap_or(lines.len());
    Some((start, end))
}

fn direct_child_indent(lines: &[String], start: usize, end: usize) -> usize {
    lines[start + 1..end]
        .iter()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| leading_spaces(line))
        .filter(|indent| *indent > 0)
        .min()
        .unwrap_or(2)
}

fn trailing_section_insertion(lines: &[String], start: usize, end: usize) -> usize {
    let mut insertion = end;
    while insertion > start + 1 {
        let line = lines[insertion - 1].trim();
        if line.is_empty() || line.starts_with('#') {
            insertion -= 1;
        } else {
            break;
        }
    }
    insertion
}

fn leading_spaces(line: &str) -> usize {
    line.chars()
        .take_while(|character| *character == ' ')
        .count()
}

fn significant_content(line: &str) -> &str {
    line.split_once('#').map_or(line, |(content, _)| content)
}

fn inline_comment_suffix(line: &str) -> String {
    line.find('#')
        .map(|index| format!(" {}", line[index..].trim()))
        .unwrap_or_default()
}

fn mapping_key_prefix(line: &str, key: &str) -> bool {
    significant_content(line.trim_start())
        .trim_end()
        .strip_prefix(key)
        .is_some_and(|suffix| suffix.starts_with(':'))
}

fn parsed_mapping_key(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let value: serde_yaml::Value = serde_yaml::from_str(trimmed).ok()?;
    let mapping = value.as_mapping()?;
    if mapping.len() != 1 {
        return None;
    }
    mapping.keys().next()?.as_str().map(str::to_owned)
}

fn render_mapping_line(indent: usize, key: &str, value: &str) -> anyhow::Result<String> {
    let mut mapping = serde_yaml::Mapping::new();
    mapping.insert(
        serde_yaml::Value::String(key.to_string()),
        serde_yaml::Value::String(value.to_string()),
    );
    let rendered = serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping))?;
    let rendered = rendered.trim_end_matches(&['\r', '\n'][..]);
    if rendered.contains('\n') {
        anyhow::bail!("abbreviation values must fit on one YAML line");
    }
    Ok(format!("{}{}", " ".repeat(indent), rendered))
}

fn join_lines(lines: Vec<String>, trailing_newline: bool) -> String {
    let mut rendered = lines.join("\n");
    if trailing_newline {
        rendered.push('\n');
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"bot:
  token: test
permissions:
  owner_id: 1
  allowed_users: [2, 3] # keep-users-comment

tts:
  provider: msedge
abbreviations:
  ko: "không"
  "dm": "địt mẹ"
  # keep-abbreviation-comment
ai:
  enabled: false
"#;

    #[test]
    fn allowed_user_render_preserves_other_content_and_inline_comment() {
        let rendered = render_allowed_users(SAMPLE, &[2, 3, 4]).expect("render");
        assert!(rendered.contains("allowed_users: [2, 3, 4] # keep-users-comment"));
        assert!(rendered.contains("  ko: \"không\""));
        assert!(rendered.contains("# keep-abbreviation-comment"));
    }

    #[test]
    fn abbreviation_upsert_is_case_insensitive_and_preserves_other_sections() {
        let rendered = upsert_abbreviation(SAMPLE, "KO", "khum").expect("upsert");
        assert_eq!(rendered.matches("ko: khum").count(), 1);
        assert!(!rendered.contains("ko: \"không\""));
        assert!(rendered.contains("allowed_users: [2, 3] # keep-users-comment"));
        assert!(rendered.contains("# keep-abbreviation-comment"));
    }

    #[test]
    fn abbreviation_delete_removes_only_matching_key() {
        let rendered = delete_abbreviation(SAMPLE, "DM").expect("delete");
        assert!(!rendered.contains("\"dm\": \"địt mẹ\""));
        assert!(rendered.contains("ko: \"không\""));
        assert!(rendered.contains("ai:"));
    }
}
