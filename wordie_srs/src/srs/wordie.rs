use std::{str::FromStr, time::Duration};
use chrono::{DateTime, Local, Timelike, NaiveDateTime};
use lazy_static::lazy_static;
use mysql::{prelude::*, Pool, params};
use charabia::Tokenize;
use uuid::Uuid;

use crate::srs::Sentence;

use super::{SrsAlgorithm, SrsResult, Review, Difficulty};

lazy_static! {
    /// The initial intervals for new cards
    static ref INITIAL_INTERVALS: [Duration; 3] = [
        Duration::from_secs(1 * 60),
        Duration::from_secs(10 * 60),
        Duration::from_secs(24 * 60 * 60),
    ];
}

/// The default ease
const DEFAULT_EASE: f32 = 2.5;

/// The minimum ease
const MINIMUM_EASE: f32 = 1.3;

/// The easy bonus
const EASY_BONUS: f64 = 1.3;

/// The hard interval
const HARD_INTERVAL: f64 = 1.2;

/// The max number of cards in learning state at once
const MAX_LEARNING_CARDS: i32 = 10;

/// A card
#[derive(Debug)]
struct Card {
    word_id: String,
    due: Option<NaiveDateTime>,
    interval: Option<Duration>,
    review_count: i32,
    ease: f32,
}

impl Card {
    fn review(&mut self, time_now: DateTime<Local>, score: Difficulty) -> SrsResult<()> {
        // https://faqs.ankiweb.net/what-spaced-repetition-algorithm.html
        // For learning/relearning the algorithm is a bit different. We track if a card is
        // currently in the learning stage by its review count, if there's a corresponding entry in
        // INITIAL_INTERVALS that's one of the initial learning stages, once it passes out of there
        // it graduates to no longer being a new card.
        if self.review_count < INITIAL_INTERVALS.len() as i32 {
            // For cards in learning/relearning:
            // * Again moves the card back to the first stage of the new card intervals
            // * Hard repeats the current step
            // * Good moves the card to the next step, if the card was on the final step, it is
            //   converted into a review card
            // * Easy immediately converts the card into a review card
            // There are no ease adjustments for new cards.
            self.review_count = match score {
                Difficulty::Again => 0,
                Difficulty::Hard => self.review_count,
                Difficulty::Good => self.review_count + 1,
                Difficulty::Easy => INITIAL_INTERVALS.len() as i32,
            };

            let interval_index = i32::clamp(self.review_count, 0, INITIAL_INTERVALS.len() as i32 - 1);
            let new_interval = INITIAL_INTERVALS[interval_index as usize];
            let new_due = time_now + chrono::Duration::from_std(new_interval)?;

            self.interval = Some(new_interval);
            self.due = Some(new_due.naive_utc());
        }
        else {
            // For cards that have graduated learning:
            // * Again puts the card back into learning mode, and decreases the ease by 20%
            // * Hard multiplies the current interval by the hard interval (1.2 by default) and
            //   decreases the ease by 15%
            // * Good multiplies the current interval by the ease
            // * Easy multiplies the current interval by the ease times the easy bonus (1.3 by
            //   default) and increases the ease by 15%
            let (new_interval, new_ease, new_review_count) = match score {
                Difficulty::Again => {
                    (INITIAL_INTERVALS[0], self.ease - 0.2, 0)
                },
                Difficulty::Hard => {
                    let new_interval = Self::mul_duration(self.interval.unwrap(), HARD_INTERVAL);
                    (new_interval, self.ease - 0.15, self.review_count + 1)
                },
                Difficulty::Good => {
                    let new_interval = Self::mul_duration(self.interval.unwrap(), self.ease as f64);
                    (new_interval, self.ease, self.review_count + 1)
                },
                Difficulty::Easy => {
                    let new_interval = Self::mul_duration(self.interval.unwrap(), self.ease as f64 * EASY_BONUS);
                    (new_interval, self.ease + 0.15, self.review_count + 1)
                },
            };

            let new_due = time_now + chrono::Duration::from_std(new_interval)?;

            self.interval = Some(new_interval);
            self.due = Some(new_due.naive_utc());
            self.ease = f32::max(MINIMUM_EASE, new_ease);
            self.review_count = new_review_count;
        }

        Ok(())
    }

    fn mul_duration(duration: Duration, multiplier: f64) -> Duration {
        let new_interval_secs = duration.as_secs() as f64 * multiplier;
        Duration::from_secs(new_interval_secs as u64)
    }
}

/// Wordie srs algorithm, version 1
pub struct WordieSrsAlgorithm {
    pool: Pool,
    new_card_limit: i32,
    // TODO: should store this in db, or it doesn't persist app restarts
    cards_learned_today: i32,
    cards_reviewed_today: i32,
    local_time: DateTime<Local>,
}

impl WordieSrsAlgorithm {
    /// Connect to a database and create a new WordieSrsAlgorithm
    pub fn new(db_url: &str, new_card_limit: i32) -> SrsResult<Self> {
        let pool = Pool::new(db_url)?;

        Ok(WordieSrsAlgorithm {
            pool,
            new_card_limit,
            cards_learned_today: 0,
            cards_reviewed_today: 0,
            local_time: Local::now(),
        })
    }

    fn get_next_due(&self) -> SrsResult<Option<Review>> {
        let mut conn = self.pool.get_conn()?;

        let midnight = (self.local_time + chrono::Duration::days(1))
            .with_hour(0).unwrap()
            .with_minute(0).unwrap()
            .with_second(0).unwrap()
            .with_nanosecond(0).unwrap();

        let result = conn.exec_map(
            r"
                -- Find a sentence to review: Get all the sentences with words due today, and order them
                -- by how many words in each one are due today to find the one most worth reviewing
                SELECT sentence_words.sentence_id, sentences.text, count(cards.word_id) as words_due
                FROM cards
                INNER JOIN sentence_words ON sentence_words.word_id = cards.word_id
                LEFT JOIN (
                    -- Get all the sentences with unlearned words
                    SELECT DISTINCT sentence_words.sentence_id
                    FROM sentence_words
                    INNER JOIN cards ON sentence_words.word_id = cards.word_id
                    WHERE cards.due IS NULL
                ) sentences_with_unlearned_words ON sentences_with_unlearned_words.sentence_id = sentence_words.sentence_id
                INNER JOIN sentences ON sentences.id = sentence_words.sentence_id
                WHERE sentences_with_unlearned_words.sentence_id IS NULL
                   && cards.due IS NOT NULL
                   && cards.due < :latest_time
                GROUP BY sentence_words.sentence_id
                ORDER BY words_due DESC
                LIMIT 1
            ",
            params! {
                "latest_time" => midnight.naive_utc()
            },
            |(sentence_id, text, words_due) : (String, String, i32)| {
                Review::Due {
                    sentence: Sentence {
                        id: Uuid::from_str(sentence_id.as_str()).unwrap(),
                        text,
                    },
                    words_due,
                }
            })?;

        Ok(result.into_iter().next())
    }

    fn get_next_new(&self) -> SrsResult<Option<Review>> {
        // If there are too many cards in learning, let user do some reviews first
        let learning_count = self.cards_in_learning_count()?;
        if learning_count >= MAX_LEARNING_CARDS {
            log::info!("Too many cards in learning ({learning_count}) to get a new card");
            return Ok(None);
        }
        else {
            log::info!("Only ({learning_count}) cards in learning, getting a new card");
        }

        if self.cards_learned_today >= self.new_card_limit {
            log::info!("at new word limit, cards learned: {}, limit: {}", self.cards_learned_today, self.new_card_limit);
            return Ok(None);
        }

        let mut conn = self.pool.get_conn()?;

        let result = conn.query_map(
            r"
                -- Find a new sentence to learn: First we get all pairs of (sentence_id, word_id) where word_id
                -- is an unlearned word. Then we group by the sentence id and count the unknown words in each one
                -- to find the most i+1 sentence to learn.
                SELECT sentences_with_unlearned.sentence_id, sentences.text, count(sentences_with_unlearned.word_id)
                FROM (
                    -- Get all sentences with unlearned words, along with the unlearned words in them
                    SELECT sentence_words.sentence_id, cards.word_id
                    FROM cards
                    INNER JOIN sentence_words ON sentence_words.word_id = cards.word_id
                    WHERE cards.due IS NULL
                    ORDER BY cards.added_order ASC
                ) sentences_with_unlearned
                INNER JOIN sentences ON sentences.id = sentences_with_unlearned.sentence_id
                GROUP BY sentences_with_unlearned.sentence_id
                ORDER BY count(sentences_with_unlearned.word_id)
                LIMIT 1
            ",
            |(sentence_id, text, unknown_words) : (String, String, i32)| {
                Review::New {
                    sentence: Sentence {
                        id: Uuid::from_str(sentence_id.as_str()).unwrap(),
                        text,
                    },
                    unknown_words,
                }
            })?;

        Ok(result.into_iter().next())
    }

    fn cards_in_learning_count(&self) -> SrsResult<i32> {
        let mut conn = self.pool.get_conn()?;

        let midnight = (self.local_time + chrono::Duration::days(1))
            .with_hour(0).unwrap()
            .with_minute(0).unwrap()
            .with_second(0).unwrap()
            .with_nanosecond(0).unwrap();

        Ok(conn.exec_first(
            r"SELECT count(*)
              FROM cards
              WHERE cards.review_count < :max_review_count
                 && cards.due IS NOT NULL
                 && cards.due < :latest_time",
            params! {
                "max_review_count" => INITIAL_INTERVALS.len(),
                "latest_time" => midnight.naive_utc(),
            })?
            .unwrap_or(0))
    }
}

impl SrsAlgorithm for WordieSrsAlgorithm {
    fn reinitialize_db(&mut self) -> SrsResult<()> {
        log::info!("Reinitializing database");

        // Drop all tables
        self.pool.get_conn()?.query_drop("DROP TABLE IF EXISTS sentence_words, cards, sentences, words, reviews")?;

        // Initialise db
        self.initialize_db()
    }

    fn initialize_db(&mut self) -> SrsResult<()> {
        log::info!("Initializing database");

        let mut conn = self.pool.get_conn()?;

        // Recreate tables
        conn.query_drop(r"
            CREATE TABLE IF NOT EXISTS sentences (
                id CHAR(36) NOT NULL,
                text TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
                PRIMARY KEY (id)
            )
        ")?;

        conn.query_drop(r"
            CREATE TABLE IF NOT EXISTS words (
                id CHAR(36) NOT NULL,
                word VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL UNIQUE,
                PRIMARY KEY (id)
            )
        ")?;

        conn.query_drop(r"
            CREATE TABLE IF NOT EXISTS sentence_words (
                sentence_id CHAR(36) NOT NULL,
                word_id CHAR(36) NOT NULL,
                FOREIGN KEY (sentence_id) REFERENCES sentences(id),
                FOREIGN KEY (word_id) REFERENCES words(id),
                PRIMARY KEY (word_id, sentence_id)
            )
        ")?;

        conn.query_drop(r"
            CREATE TABLE IF NOT EXISTS cards (
                word_id CHAR(36) NOT NULL,
                review_count INT NOT NULL,
                ease FLOAT NOT NULL,
                `interval` TIME,
                due DATETIME,
                added_order INT NOT NULL,
                FOREIGN KEY (word_id) REFERENCES words(id),
                PRIMARY KEY (word_id)
            )
        ")?;

        conn.query_drop(r"
            CREATE TABLE IF NOT EXISTS reviews (
                word_id CHAR(36) NOT NULL,
                review_date DATETIME NOT NULL,
                FOREIGN KEY (word_id) REFERENCES words(id)
            )
        ")?;

        Ok(())
    }

    fn set_time_now(&mut self, time: chrono::DateTime<chrono::Local>) {
        log::info!("Setting current time to {time:?}");
        self.local_time = time;
    }

    fn reset_daily_limits(&mut self) {
        log::info!("Resetting daily card limits");
        self.cards_learned_today = 0;
    }

    fn add_sentences(&mut self, sentences: &[super::Sentence]) -> SrsResult<()> {
        let mut conn = self.pool.get_conn()?;

        // Tokenize sentences, and then add them to the db
        for sentence in sentences.iter() {
            // Tokenize sentence into words
            let words = sentence.text
                .as_str()
                .tokenize()
                .filter(|token| token.is_word())
                .map(|token| token.lemma.to_string())
                .collect::<Vec<String>>();

            // Add new words to database
            conn.exec_batch("INSERT IGNORE INTO words (id, word) VALUES (:id, :word)",
                words.iter().map(|word| params! {
                    "id" => Uuid::new_v4().to_string(),
                    "word" => word.as_str(),
                }))?;

            // Get words with proper ids (they might have existed in the db with an id already).
            // TODO: Annoyingly, there's no way to parameterise the IN (?) part of the query, and
            // you have to build the query with the words in it instead. This probably opens us up
            // to SQL injection.
            let query = {
                let mut query = "SELECT id FROM words WHERE word in (".to_string();

                for (i, word) in words.iter().enumerate() {
                    if i != 0 {
                        query.push(',');
                    }

                    query.push('"');
                    query.push_str(word);
                    query.push('"');
                }

                query.push(')');

                query
            };

            let word_ids: Vec<String> = conn.query(query)?;

            // Insert sentence
            let sentence_id = sentence.id.to_string();
            conn.exec_drop("INSERT INTO sentences (id, text) VALUES (:id, :text)",
                params! {
                    "id" => sentence_id.as_str(),
                    "text" => sentence.text.as_str(),
                })?;

            // Insert sentence words
            conn.exec_batch("INSERT INTO sentence_words (sentence_id, word_id) VALUES (:sentence_id, :word_id)",
                word_ids.iter().map(|word| params! {
                    "sentence_id" => sentence_id.as_str(),
                    "word_id" => word,
                }))?;

            // Insert cards
            conn.exec_batch(
                r"INSERT IGNORE INTO cards (word_id, review_count, ease, added_order)
                  VALUES (:word_id, :review_count, :ease, :added_order)",
                word_ids.iter().enumerate().map(|(i, w)| params! {
                    "word_id" => w,
                    "review_count" => 0,
                    "ease" => DEFAULT_EASE,
                    "added_order" => i,
                })
            )?;
        }
        Ok(())
    }

    fn get_next_card(&self) -> SrsResult<Option<super::Review>> {
        let next_card = self.get_next_new()?
            .or(self.get_next_due()?);

        Ok(next_card)
    }

    fn review(&mut self, review: super::Review, score: super::Difficulty) -> SrsResult<()> {
        let mut conn = self.pool.get_conn()?;

        // Get cards for words in the sentence
        let mut cards = conn.exec_map(
            r"SELECT cards.word_id, cards.review_count, cards.ease, cards.interval, cards.due
              FROM sentence_words
              INNER JOIN cards ON cards.word_id = sentence_words.word_id
              WHERE sentence_words.sentence_id = :sentence_id",
            params! { "sentence_id" => review.sentence().id.to_string() },
            |(word_id, review_count, ease, interval, due) : (String, i32, f32, Option<Duration>, Option<NaiveDateTime>)| Card {
                word_id,
                review_count,
                ease,
                interval,
                due,
            })?;

        // Mark each word as reviewed
        for card in cards.iter_mut() {
            // Increment reviewed count
            self.cards_reviewed_today += 1;

            // If this is a new card, increment new cards count
            if card.due.is_none() {
                log::info!("Learnt new card");
                self.cards_learned_today += 1;
            }

            // Review card
            card.review(self.local_time, score)?;

            // Update card in db
            conn.exec_drop(
                r"UPDATE cards
                  SET cards.review_count = :review_count,
                      cards.ease = :ease,
                      cards.interval = :interval,
                      cards.due = :due
                  WHERE cards.word_id = :id",
                params! {
                    "id" => card.word_id.as_str(),
                    "review_count" => card.review_count,
                    "ease" => card.ease,
                    "interval" => card.interval.unwrap(),
                    "due" => card.due.unwrap(),
                })?;
        }

        Ok(())
    }

    fn cards_learned_today(&self) -> i32 {
        self.cards_learned_today
    }

    fn cards_reviewed_today(&self) -> i32 {
        self.cards_reviewed_today
    }

    fn get_suggested_sentences(&self, new_word_limit: i32) -> SrsResult<Vec<(Sentence, Vec<String>)>> {
        let mut conn = self.pool.get_conn()?;

        log::info!("Getting recommended i+{new_word_limit} sentences");

        let res: Vec<(String, String, String)> = conn.query(
            format!(r"
                -- Get a list of sentences and unknown words for sentences that are up to i+n
                SELECT sentences.id, sentences.text, words.word
                FROM (
                    SELECT sentence_words.sentence_id, count(sentence_words.word_id) as unknown_words
                    FROM cards
                    INNER JOIN sentence_words ON sentence_words.word_id = cards.word_id
                    WHERE cards.due IS NULL
                    GROUP BY sentence_words.sentence_id
                ) unlearned_sentences
                INNER JOIN sentence_words ON sentence_words.sentence_id = unlearned_sentences.sentence_id
                INNER JOIN sentences ON sentences.id = unlearned_sentences.sentence_id
                INNER JOIN words ON words.id = sentence_words.word_id
                INNER JOIN cards ON cards.word_id = sentence_words.word_id
                WHERE unlearned_sentences.unknown_words <= {new_word_limit}
                   && cards.due IS NULL
                ORDER BY unlearned_sentences.unknown_words
            "))?;

        let mut ret = Vec::new();
        let mut last_sentence_id: Option<String> = None;

        for (sentence_id, sentence_text, word) in res.iter() {
            if last_sentence_id.is_none() || last_sentence_id.as_ref().unwrap() != sentence_id {
                let sentence = Sentence { id: Uuid::from_str(sentence_id.as_str()).unwrap(), text: sentence_text.clone() };
                ret.push((sentence, Vec::new()));
                last_sentence_id = Some(sentence_id.clone());
            }

            ret.last_mut().unwrap().1.push(word.clone());
        };

        Ok(ret)
    }
}
