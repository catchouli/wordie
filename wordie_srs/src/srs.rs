pub mod anki;
pub mod wordie;

use chrono::{Local, DateTime};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use strum_macros::EnumIter;

/// A result type that boxes errors to a Box<dyn Error>
pub type SrsResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Type for a review
#[derive(Debug, Clone)]
pub enum Review {
    New { sentence: Sentence, unknown_words: i32 },
    Due { sentence: Sentence, words_due: i32 },
}

impl Review {
    pub fn sentence(&self) -> &Sentence {
        match &self {
            Review::New { sentence, .. } => &sentence,
            Review::Due { sentence, ..} => &sentence,
        }
    }
}

/// Review difficulties
#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone, EnumIter)]
pub enum Difficulty {
    Again = 0,
    Hard = 1,
    Good = 2,
    Easy = 3
}

/// Type for a sentence in the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sentence {
    pub id: Uuid,
    pub text: String,
}

/// Trait for an SRS algorithm
pub trait SrsAlgorithm {
    /// Clear the db, resetting the db structure and clearing all data
    fn reinitialize_db(&mut self) -> SrsResult<()>;

    /// Initialise the db
    fn initialize_db(&mut self) -> SrsResult<()>;

    /// Add sentences
    fn add_sentences(&mut self, sentences: &[Sentence]) -> SrsResult<()>;

    /// Get next card (new or review, depending on settings and algorithm)
    fn get_next_card(&self) -> SrsResult<Option<Review>>;

    /// Complete a review
    fn review(&mut self, review: Review, difficulty: Difficulty) -> SrsResult<()>;

    /// Get the number of cards learned today
    fn cards_learned_today(&self) -> i32;

    /// Get the number of cards reviewed today
    fn cards_reviewed_today(&self) -> i32;

    /// Reset daily limits
    fn reset_daily_limits(&mut self);

    /// Set the current time
    fn set_time_now(&mut self, time: DateTime<Local>);

    /// Get suggested sentences by new word limit
    fn get_suggested_sentences(&self, new_word_limit: i32) -> SrsResult<Vec<(Sentence, Vec<String>)>>;
}
