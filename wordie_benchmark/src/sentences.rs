use std::{error::Error, io::Cursor};
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use uuid::Uuid;

use wordie_srs::srs::Sentence;

/// The sentences.csv file
const CORE_6K: &'static [u8] = include_bytes!("../../resources/sentences.csv");

/// Sentence from the kore 6k sentences.csv, so many columns....
#[derive(Debug, Serialize, Deserialize)]
struct CoreSentence {
    core_index: i32,
    vocab_ko_index: i32,
    sent_ko_index: i32,
    new_opt_voc_index: i32,
    opt_voc_index: i32,
    opt_sen_index: i32,
    jlpt: String,
    vocab_expression: String,
    vocab_kana: String,
    vocab_meaning: String,
    vocab_sound_local: String,
    vocab_pos: String,
    sentence_expression: String,
    sentence_kana: String,
    sentence_meaning: String,
    sentence_sound_local: String,
    sentence_image_local: String,
    vocab_furigana: String,
    sentence_furigana: String,
    sentence_cloze: String,
}

impl From<CoreSentence> for Sentence {
    fn from(cs: CoreSentence) -> Self {
        Sentence {
            id: Uuid::new_v4(),
            text: cs.sentence_expression,
        }
    }
}

/// Load sentences from a csv in a &[u8] up to an (optional) maximum number
fn from_csv<T: Into<Sentence> + DeserializeOwned>(csv: &[u8], max_sentences: Option<usize>) -> Result<Vec<Sentence>, Box<dyn Error>> {
    let cursor = Cursor::new(csv);
    let mut reader = csv::Reader::from_reader(cursor);

    let sentence_iter = reader
        .deserialize()
        .map(|record| {
            let record: T = record?;
            Ok(record.into())
        });

    if let Some(max) = max_sentences {
        sentence_iter.take(max).collect()
    }
    else {
        sentence_iter.collect()
    }
}

/// Load core 6k sentences
pub fn core_6k(max_sentences: Option<usize>) -> Result<Vec<Sentence>, Box<dyn Error>> {
    from_csv::<CoreSentence>(CORE_6K, max_sentences)
}
