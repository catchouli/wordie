pub mod anki;
pub mod wordie;

use chrono::{Local, DateTime};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A result type that boxes errors to a Box<dyn Error>
type SrsResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Type for a review
#[derive(Debug, Clone)]
pub enum Review {
    New(Sentence),
    Due(Sentence),
}

impl Review {
    pub fn sentence(&self) -> &Sentence {
        match &self {
            Review::New(sentence) => &sentence,
            Review::Due(sentence) => &sentence,
        }
    }
}

/// Review difficulties
#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone)]
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

    /// Get the number of cards learnt today
    fn cards_learnt_today(&self) -> i32;

    /// Reset daily limits
    fn reset_daily_limits(&mut self);

    /// Set the current time
    fn set_time_now(&mut self, time: DateTime<Local>);
}
