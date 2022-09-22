use std::str::FromStr;
use std::time::Duration;
use chrono::{NaiveDateTime, Timelike, Local, DateTime};
use lazy_static::lazy_static;
use uuid::Uuid;

use mysql::{Pool, prelude::Queryable, params};
use super::{SrsAlgorithm, SrsResult, Sentence, Review, Difficulty};

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

/// An srs card
struct Card {
    id: String,
    due: Option<NaiveDateTime>,
    interval: Option<Duration>,
    review_count: i32,
    ease: f32,
}

type CardRecord = (Option<NaiveDateTime>, Option<Duration>, i32, f32);

impl Card {
    fn new(id: String, (due, interval, review_count, ease): CardRecord) -> Self {
        Self {
            id,
            due,
            interval,
            review_count,
            ease,
        }
    }

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

/// Anki-style spaced repetition implementation
pub struct AnkiSrsAlgorithm {
    pool: Pool,
    new_card_limit: i32,
    // TODO: should store this in db, or it doesn't persist app restarts
    cards_learnt_today: i32,
    local_time: DateTime<Local>,
}

impl AnkiSrsAlgorithm {
    /// Connect to a database and create a new AnkiSrsAlgorithm
    pub fn new(db_url: &str, new_card_limit: i32) -> SrsResult<Self> {
        let pool = Pool::new(db_url)?;

        Ok(AnkiSrsAlgorithm {
            pool,
            new_card_limit,
            cards_learnt_today: 0,
            local_time: Local::now(),
        })
    }

    fn get_card(&self, sentence_id: &str) -> SrsResult<Card> {
        let mut conn = self.pool.get_conn()?;

        let record: CardRecord = conn.exec_first(
            r"SELECT cards.due, cards.interval, cards.review_count, cards.ease
              FROM cards
              WHERE cards.sentence_id = :sentence_id",
              params! { "sentence_id" => sentence_id.to_string() }
            )?
            .expect(&format!("No such sentence {}", sentence_id));

        Ok(Card::new(sentence_id.to_string(), record))
    }

    fn update_card(&mut self, card: Card) -> SrsResult<()> {
        let mut conn = self.pool.get_conn()?;

        conn.exec_drop(
            r"UPDATE cards
              SET cards.due = :due, cards.interval = :interval, cards.review_count = :review_count, cards.ease = :ease
              WHERE cards.sentence_id = :sentence_id",
              params! {
                "sentence_id" => card.id,
                "due" => card.due.unwrap(),
                "interval" => card.interval.unwrap(),
                "review_count" => card.review_count,
                "ease" => card.ease,
              })?;

        Ok(())
    }

    fn get_next_due(&self) -> SrsResult<Option<Review>> {
        let mut conn = self.pool.get_conn()?;

        let midnight = (self.local_time + chrono::Duration::days(1))
            .with_hour(0).unwrap()
            .with_minute(0).unwrap()
            .with_second(0).unwrap()
            .with_nanosecond(0).unwrap();

        let result = conn.exec_first(
            r"SELECT cards.sentence_id, sentences.text
              FROM cards
              INNER JOIN sentences ON cards.sentence_id = sentences.id
              WHERE cards.due IS NOT NULL AND cards.due < :latest_time
              ORDER BY cards.due, cards.added_order ASC
              LIMIT 1",
            params! {
                "latest_time" => midnight.naive_utc()
            })?
            .map(|(id, text): (String, String)| {
                Review::Due(Sentence {
                    id: Uuid::from_str(&id).unwrap(),
                    text
                })
            });

        let results = result.iter().next().map(|review| review.clone());

        Ok(results)
    }

    fn get_next_new(&self) -> SrsResult<Option<Review>> {
        if self.cards_learnt_today >= self.new_card_limit {
            return Ok(None);
        }

        let mut conn = self.pool.get_conn()?;

        let result = conn.query_map(
            r"SELECT cards.sentence_id, sentences.text
              FROM cards
              INNER JOIN sentences ON cards.sentence_id = sentences.id
              WHERE cards.due IS NULL
              ORDER BY cards.added_order ASC
              LIMIT 1",
            |(id, text): (String, String)| {
                Review::New(Sentence {
                    id: Uuid::from_str(&id).unwrap(),
                    text
                })
            })?;

        Ok(result.into_iter().next())
    }
}

impl SrsAlgorithm for AnkiSrsAlgorithm {
    fn reinitialize_db(&mut self) -> SrsResult<()> {
        log::info!("Reinitializing database");

        // Drop all tables
        self.pool.get_conn()?.query_drop("DROP TABLE IF EXISTS sentences, cards")?;

        // Initialise db
        self.initialize_db()
    }

    fn initialize_db(&mut self) -> SrsResult<()> {
        log::info!("Initializing database");

        let mut conn = self.pool.get_conn()?;

        // Recreate tables
        conn.query_drop(r"
            CREATE TABLE IF NOT EXISTS sentences (
                `id` CHAR(36) NOT NULL,
                `text` TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
                PRIMARY KEY (`id`)
            )
        ")?;

        conn.query_drop(r"
            CREATE TABLE IF NOT EXISTS cards (
                `sentence_id` CHAR(36) NOT NULL,
                `review_count` INT NOT NULL,
                `ease` FLOAT NOT NULL,
                `interval` TIME,
                `due` DATETIME,
                `added_order` INT NOT NULL,
                PRIMARY KEY (`sentence_id`)
            )
        ")?;

        Ok(())
    }

    fn add_sentences(&mut self, sentences: &[Sentence]) -> SrsResult<()> {
        log::info!("Adding {} sentences", sentences.len());

        let mut conn = self.pool.get_conn()?;

        conn.exec_batch(
            r"INSERT INTO sentences (id, text)
              VALUES (:id, :text)",
            sentences.iter().map(|s| params! {
                "id" => s.id.to_string(),
                "text" => &s.text
            })
        )?;

        conn.exec_batch(
            r"INSERT INTO cards (sentence_id, review_count, ease, added_order)
              VALUES (:sentence_id, :review_count, :ease, :added_order)",
            sentences.iter().enumerate().map(|(i, s)| params! {
                "sentence_id" => s.id.to_string(),
                "review_count" => 0,
                "ease" => DEFAULT_EASE,
                "added_order" => i,
            })
        )?;

        Ok(())
    }

    fn get_next_card(&self) -> SrsResult<Option<Review>> {
        Ok(self.get_next_new()?.or(self.get_next_due()?))
    }

    // TODO: might be better if we get the record that matches the review from the database,
    // and if it doesn't match anymore then maybe this review is out of date, so we return an
    // error
    fn review(&mut self, review: Review, score: Difficulty) -> SrsResult<()> {
        let sentence = review.sentence();

        // Get card to review
        let mut card = self.get_card(&sentence.id.to_string())?;

        // Increment new cards learnt if this is a new card
        if card.due.is_none() {
            self.cards_learnt_today += 1;
        }

        // Review card
        card.review(self.local_time, score)?;

        // Update card
        self.update_card(card)?;
        
        Ok(())
    }

    fn reset_daily_limits(&mut self) {
        log::info!("Resetting daily card limits");
        self.cards_learnt_today = 0;
    }

    fn set_time_now(&mut self, time: DateTime<Local>) {
        log::info!("Setting current time to {time:?}");
        self.local_time = time;
    }

    fn cards_learnt_today(&self) -> i32 {
        self.cards_learnt_today
    }
}
