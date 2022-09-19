mod srs;
mod core;

use std::fs::File;
use std::io::Write;
use std::{error::Error, collections::HashMap};
use chrono::Local;
use lazy_static::lazy_static;
use rand::Rng;
use srs::{anki::AnkiSrsAlgorithm, SrsAlgorithm, Difficulty};

use crate::srs::Review;

/// The maximum number of new cards per day
const NEW_CARDS_PER_DAY: i32 = 50;

lazy_static! {
    /// Score distributions
    static ref SCORE_DISTRIBUTIONS: HashMap<Difficulty, i32> = HashMap::from([
        (Difficulty::Again, 5),
        (Difficulty::Hard, 10),
        (Difficulty::Good, 80),
        (Difficulty::Easy, 5),
    ]);

    static ref SCORE_DISTRIBUTIONS_TOTAL: i32 = SCORE_DISTRIBUTIONS.iter()
        .fold(0, |acc, (_, weight)| acc + weight);
}

fn random_difficulty() -> Difficulty {
    let value = rand::thread_rng().gen_range(0..*SCORE_DISTRIBUTIONS_TOTAL);

    let mut acc = 0;
    for (score, weight) in SCORE_DISTRIBUTIONS.iter() {
        if value >= acc && value < acc + weight {
            return *score;
        }

        acc += weight;
    }

    panic!("Internal error, got to end");
}

fn main() -> Result<(), Box<dyn Error>> {
    // Initialise logging
    env_logger::init();
    log::info!("Starting wordie");

    // Create the SrsAlgorithm
    let mut srs_algorithm = AnkiSrsAlgorithm::new("mysql://root:password@localhost:3306/wordie_anki", NEW_CARDS_PER_DAY)?;

    // Reinitialize db
    srs_algorithm.reinitialize_db()?;

    // Add sentences
    srs_algorithm.add_sentences(&core::sentences()?)?;

    // Output file
    let mut f = File::create("out.csv")?;
    writeln!(&mut f, "day,learnt,reviewed")?;

    // Do some reviews
    let actual_start = Local::now();
    for day in 0..100 {
        log::info!("Starting day {day}");
        let day_start = actual_start + chrono::Duration::days(day);
        srs_algorithm.set_time_now(day_start);

        let mut learnt = 0;
        let mut reviewed = 0;

        loop {
            // Get next new card, or next due card if there are no new cards
            let next_card = srs_algorithm.get_next_new()?
                .or(srs_algorithm.get_next_due()?);

            if let Some(review @ Review::New(_)) = next_card {
                log::info!("New card: {}", review.sentence().text);
                srs_algorithm.review(review, random_difficulty())?;
                learnt += 1;
            }
            else if let Some(review @ Review::Due(_)) = next_card {
                log::info!("Due card: {}", review.sentence().text);
                srs_algorithm.review(review, random_difficulty())?;
                reviewed += 1;
            }
            else {
                log::info!("No more sentences to review");
                break;
            }
        }

        writeln!(&mut f, "{day},{learnt},{reviewed}")?;

        srs_algorithm.reset_daily_limits();
    }

    log::info!("Exiting wordie");
    Ok(())
}
