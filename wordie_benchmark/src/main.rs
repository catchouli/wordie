mod sentences;

use std::fs::File;
use std::io::Write;
use std::{error::Error, collections::HashMap};

use rand::Rng;
use chrono::Local;
use lazy_static::lazy_static;

use wordie_srs::srs::anki::AnkiSrsAlgorithm;
use wordie_srs::srs::{SrsAlgorithm, Review, Difficulty};
use wordie_srs::srs::wordie::WordieSrsAlgorithm;

/// The srs algorithm to use
pub enum Algorithm {
    Anki,
    Wordie
}

/// The algorithm to use
const ALGORITHM_TO_USE: Algorithm = Algorithm::Wordie;

/// The maximum number of new cards per day
const NEW_CARDS_PER_DAY: i32 = 50;

/// The number of days to review for
const DAYS_TO_REVIEW: i64 = 100;

/// The max number of sentences to load
const MAX_SENTENCES: Option<usize> = None;

lazy_static! {
    /// Score distributions
    static ref SCORE_DISTRIBUTIONS: HashMap<Difficulty, i32> = HashMap::from([
        (Difficulty::Again, 5),
        (Difficulty::Hard, 10),
        (Difficulty::Good, 80),
        (Difficulty::Easy, 5),
    ]);

    /// The total weights of all the score distributions
    static ref SCORE_DISTRIBUTIONS_TOTAL: i32 = SCORE_DISTRIBUTIONS.iter()
        .fold(0, |acc, (_, weight)| acc + weight);
}

/// Pick a random difficulty based on the score distributions above
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

/// Simulate an srs algorithm
fn simulate<W: Write>(mut srs_algorithm: Box<dyn SrsAlgorithm>, mut writer: W) -> Result<(), Box<dyn Error>> {
    log::info!("Simulating srs algorithm");

    // Reinitialize db
    srs_algorithm.reinitialize_db()?;

    // Add sentences
    srs_algorithm.add_sentences(&sentences::core_6k(MAX_SENTENCES)?)?;

    // Output header row to writer
    writeln!(&mut writer, "day,learnt,reviewed")?;

    // Do some reviews
    let actual_start = Local::now();
    for day in 0..DAYS_TO_REVIEW {
        // Start day and set datetime accordingly
        log::info!("Starting day {day}");
        let day_start = actual_start + chrono::Duration::days(day);
        srs_algorithm.set_time_now(day_start);

        // Do all daily reviews
        let mut review_count = 0;
        loop {
            let next_card = srs_algorithm.get_next_card()?;

            if let Some(review @ Review::New(_)) = next_card {
                log::info!("New card: {}", review.sentence().text);
                srs_algorithm.review(review, random_difficulty())?;
                review_count += 1;
            }
            else if let Some(review @ Review::Due(_)) = next_card {
                log::info!("Due card: {}", review.sentence().text);
                srs_algorithm.review(review, random_difficulty())?;
                review_count += 1;
            }
            else {
                log::info!("No more sentences to review");
                break;
            }
        }

        // Output daily row to writer
        let learnt = srs_algorithm.cards_learnt_today();
        writeln!(&mut writer, "{day},{learnt},{review_count}")?;

        // Reset daily limits and move on to the next day
        srs_algorithm.reset_daily_limits();
    }

    log::info!("Done simulating");
    Ok(())
}

/// Entry point
fn main() -> Result<(), Box<dyn Error>> {
    // Initialise logging
    env_logger::init();
    log::info!("Starting wordie");

    // Create output file
    let mut f = File::create("out.csv")?;

    // Create the SrsAlgorithm
    let srs: Box<dyn SrsAlgorithm> = match ALGORITHM_TO_USE {
        Algorithm::Anki => Box::new(
            AnkiSrsAlgorithm::new("mysql://root:password@localhost:3306/wordie_anki", NEW_CARDS_PER_DAY)?
        ),
        Algorithm::Wordie => Box::new(
            WordieSrsAlgorithm::new("mysql://root:password@localhost:3306/wordie_wordie", NEW_CARDS_PER_DAY)?
        ),
    };

    simulate(srs, &mut f)
}
