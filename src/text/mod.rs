pub mod filter;
pub mod normalizer;

use filter::MessageFilter;

pub async fn prepare_for_tts(
    ctx: &serenity::client::Context,
    msg: &serenity::model::channel::Message,
) -> String {
    MessageFilter::clean(ctx, msg).await
}
