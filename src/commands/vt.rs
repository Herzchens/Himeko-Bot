use super::owner_config::{self, AbbreviationMutation};
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Quản lý danh sách từ viết tắt TTS
#[poise::command(
    slash_command,
    subcommands("add", "remove", "list"),
    subcommand_required
)]
pub async fn vt(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Thêm hoặc cập nhật một từ viết tắt
#[poise::command(slash_command)]
pub async fn add(
    ctx: Context<'_>,
    #[description = "Từ viết tắt, ví dụ: ko"]
    #[rename = "abbreviation"]
    abbreviation: String,
    #[description = "Từ/cụm từ đầy đủ, ví dụ: không"]
    #[rename = "expansion"]
    expansion: String,
) -> Result<(), Error> {
    if !owner_config::require_owner(ctx).await? {
        return Ok(());
    }

    let abbreviation = abbreviation.trim().to_lowercase();
    let expansion = expansion.trim().to_string();

    if abbreviation.is_empty()
        || abbreviation.chars().any(char::is_whitespace)
        || abbreviation.contains('\r')
        || abbreviation.contains('\n')
        || abbreviation.chars().count() > 100
    {
        ctx.send(
            CreateReply::default()
                .content("❌ Từ viết tắt phải là một token tối đa 100 ký tự, không chứa khoảng trắng hoặc xuống dòng.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    if expansion.is_empty()
        || expansion.contains('\r')
        || expansion.contains('\n')
        || expansion.chars().count() > 1000
    {
        ctx.send(
            CreateReply::default()
                .content("❌ Từ gốc phải có 1-1000 ký tự và không chứa xuống dòng.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    match owner_config::add_abbreviation(
        ctx.data(),
        ctx.author().id.get(),
        &abbreviation,
        &expansion,
    )
    .await
    {
        Ok(AbbreviationMutation::Added) => {
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "✅ Đã thêm `{}` → `{}` và áp dụng ngay.",
                        escape_inline_code(&abbreviation),
                        escape_inline_code(&expansion)
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(AbbreviationMutation::Updated { previous }) => {
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "✅ Đã cập nhật `{}`: `{}` → `{}` và áp dụng ngay.",
                        escape_inline_code(&abbreviation),
                        escape_inline_code(&previous),
                        escape_inline_code(&expansion)
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(AbbreviationMutation::Unchanged) => {
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "ℹ️ `{}` đã được map tới `{}`.",
                        escape_inline_code(&abbreviation),
                        escape_inline_code(&expansion)
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(other) => {
            tracing::warn!(?other, abbreviation = %abbreviation, "unexpected vt-add result");
            ctx.send(
                CreateReply::default()
                    .content(
                        "❌ Không thể cập nhật từ viết tắt do trạng thái config không mong đợi.",
                    )
                    .ephemeral(true),
            )
            .await?;
        }
        Err(error) => {
            tracing::error!(abbreviation = %abbreviation, %error, "failed to add abbreviation");
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Không thể cập nhật từ viết tắt: {error}"))
                    .ephemeral(true),
            )
            .await?;
        }
    }

    Ok(())
}

/// Xóa một từ viết tắt
#[poise::command(slash_command)]
pub async fn remove(
    ctx: Context<'_>,
    #[description = "Từ viết tắt cần xóa"]
    #[rename = "abbreviation"]
    abbreviation: String,
) -> Result<(), Error> {
    if !owner_config::require_owner(ctx).await? {
        return Ok(());
    }

    let abbreviation = abbreviation.trim().to_lowercase();
    if abbreviation.is_empty()
        || abbreviation.chars().any(char::is_whitespace)
        || abbreviation.contains('\r')
        || abbreviation.contains('\n')
    {
        ctx.send(
            CreateReply::default()
                .content("❌ Từ viết tắt không hợp lệ.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    match owner_config::remove_abbreviation(ctx.data(), ctx.author().id.get(), &abbreviation).await
    {
        Ok(AbbreviationMutation::Removed { previous }) => {
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "✅ Đã xóa `{}` → `{}` và áp dụng ngay.",
                        escape_inline_code(&abbreviation),
                        escape_inline_code(&previous)
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(AbbreviationMutation::NotFound) => {
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "ℹ️ Không tìm thấy từ viết tắt `{}`.",
                        escape_inline_code(&abbreviation)
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(other) => {
            tracing::warn!(?other, abbreviation = %abbreviation, "unexpected vt-remove result");
            ctx.send(
                CreateReply::default()
                    .content("❌ Không thể xóa từ viết tắt do trạng thái config không mong đợi.")
                    .ephemeral(true),
            )
            .await?;
        }
        Err(error) => {
            tracing::error!(abbreviation = %abbreviation, %error, "failed to remove abbreviation");
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Không thể xóa từ viết tắt: {error}"))
                    .ephemeral(true),
            )
            .await?;
        }
    }

    Ok(())
}

/// Liệt kê toàn bộ từ viết tắt đang hoạt động
#[poise::command(slash_command)]
pub async fn list(ctx: Context<'_>) -> Result<(), Error> {
    if !owner_config::require_owner(ctx).await? {
        return Ok(());
    }

    let runtime = ctx.data().runtime_snapshot().await;
    let mut entries: Vec<_> = runtime.config.abbreviations.iter().collect();
    entries.sort_by_key(|(key, _)| *key);

    if entries.is_empty() {
        ctx.send(
            CreateReply::default()
                .content("ℹ️ Danh sách từ viết tắt hiện đang trống.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let lines: Vec<String> = entries
        .into_iter()
        .map(|(key, value)| {
            format!(
                "• `{}` → `{}`",
                escape_inline_code(key),
                escape_inline_code(value)
            )
        })
        .collect();
    let pages = paginate(&lines, 1850);

    for (index, page) in pages.iter().enumerate() {
        let heading = if pages.len() == 1 {
            format!("📚 **Từ viết tắt ({})**\n", lines.len())
        } else {
            format!(
                "📚 **Từ viết tắt ({}) — {}/{}**\n",
                lines.len(),
                index + 1,
                pages.len()
            )
        };
        ctx.send(
            CreateReply::default()
                .content(format!("{heading}{page}"))
                .ephemeral(true),
        )
        .await?;
    }

    Ok(())
}

fn escape_inline_code(value: &str) -> String {
    value.replace('`', "ˋ")
}

fn paginate(lines: &[String], max_chars: usize) -> Vec<String> {
    let mut pages = Vec::new();
    let mut current = String::new();

    for line in lines {
        let needed = line.chars().count() + usize::from(!current.is_empty());
        if !current.is_empty() && current.chars().count() + needed > max_chars {
            pages.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        pages.push(current);
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::paginate;

    #[test]
    fn vt_list_pagination_keeps_all_lines() {
        let lines = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];
        let pages = paginate(&lines, 12);
        assert_eq!(pages, vec!["first\nsecond", "third"]);
    }
}
