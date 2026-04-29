pub mod filter;
pub mod normalizer;

use filter::MessageFilter;
use normalizer::Normalizer;

pub fn prepare_for_tts(raw: &str, normalizer: &Normalizer) -> String {
    let filtered = MessageFilter::clean(raw);
    normalizer.expand(&filtered)
}
