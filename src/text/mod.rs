pub mod filter;
pub mod normalizer;

use filter::MessageFilter;
use normalizer::Normalizer;

pub async fn prepare_for_tts(ctx: &serenity::client::Context, msg: &serenity::model::channel::Message, normalizer: &Normalizer) -> String {
    let filtered = MessageFilter::clean(ctx, msg).await;
    let expanded = normalizer.expand(&filtered);
    expanded.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
}
