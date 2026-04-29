pub mod filter;
pub mod normalizer;

use filter::MessageFilter;
use normalizer::Normalizer;

pub fn prepare_for_tts(raw: &str, normalizer: &Normalizer) -> String {
    let filtered = MessageFilter::clean(raw);
    let expanded = normalizer.expand(&filtered);
    expanded.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
}
