use std::{error::Error, io::Cursor};
use serde::{Serialize, Deserialize};
use crate::srs::Sentence;
use uuid::Uuid;

/// The sentences.csv file
const FILE_SENTENCES: &'static [u8] = include_bytes!("../sentences_100.csv");

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

/// Load sentences
pub fn sentences() -> Result<Vec<Sentence>, Box<dyn Error>> {
    let cursor = Cursor::new(FILE_SENTENCES);
    let mut rdr = csv::Reader::from_reader(cursor);

    rdr.deserialize()
        .map(|record| {
            let record: CoreSentence = record?;

            Ok(Sentence {
                id: Uuid::new_v4(),
                text: record.sentence_expression,
                word: record.vocab_expression,
            })
        })
        .collect()
}

