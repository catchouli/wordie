use std::collections::HashSet;

use eframe::egui;
use egui::{RichText, Color32, Ui, FontDefinitions, FontData};
use wordie_srs::srs::{SrsAlgorithm, SrsResult, Review, Difficulty, Sentence};
use wordie_srs::srs::wordie::WordieSrsAlgorithm;
use strum::IntoEnumIterator;

/// The db url
const DB_URL: &'static str = "mysql://root:password@localhost:3306/wordie_app";

/// The number of new cards per day
const NEW_CARDS_PER_DAY: i32 = 50;

/// The maximum number of new cards per sentence
const MAX_NEW_CARDS_PER_SENTENCE: i32 = 1;

/// Max suggested sentences to show
const MAX_SUGGESTED_SENTENCES: usize = 5;

/// Entry point
fn main() {
    // Initialise logging
    env_logger::init();
    log::info!("Starting wordie");

    // Create gui
    let mut native_options = eframe::NativeOptions::default();
    native_options.initial_window_size = Some(egui::Vec2 { x: 500.0, y: 500.0 });
    eframe::run_native("Wordie App", native_options, Box::new(|cc| Box::new(WordieApp::new(cc).unwrap())));
}

/// Trait for screens in the app
trait WordieAppScreen {
    fn update(&mut self, app: &mut WordieApp, ctx: &egui::Context, frame: &mut eframe::Frame);
}

/// Wordie app main state
struct WordieApp {
    screens: Vec<Box<dyn WordieAppScreen>>,
    push_pop_actions: Vec<PushPopAction>,
    srs_algorithm: Box<dyn SrsAlgorithm>,
}

/// An enum for deferring screen pushes/pops, so we don't have to mutate the list of screens while
/// also updating one of them.
enum PushPopAction {
    PushScreen(Box<dyn WordieAppScreen>),
    PopScreen,
}

impl WordieApp {
    fn new(cc: &eframe::CreationContext<'_>) -> SrsResult<Self> {
        let mut srs_algorithm = Box::new(WordieSrsAlgorithm::new(DB_URL, NEW_CARDS_PER_DAY)?);
        srs_algorithm.initialize_db()?;

        cc.egui_ctx.set_fonts({
            let mut fonts = FontDefinitions::default();

            fonts.font_data.insert("noto".to_owned(),
                FontData::from_static(include_bytes!("../../resources/noto.otf")));

            fonts.families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(0, "noto".to_owned());

            fonts
        });

        Ok(Self {
            screens: vec![Box::new(MainScreen::default())],
            push_pop_actions: Default::default(),
            srs_algorithm,
        })
    }

    fn push_screen<T: WordieAppScreen + Default + 'static>(&mut self) {
        self.push_pop_actions.push(PushPopAction::PushScreen(Box::new(T::default())));
    }

    fn pop_screen(&mut self) {
        self.push_pop_actions.push(PushPopAction::PopScreen);
    }

    fn heading(ui: &mut Ui, text: &str) {
        ui.heading(RichText::new(text)
                   .color(Color32::WHITE)
                   .size(32.0));
    }
}

impl eframe::App for WordieApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Take self.screens temporarily so we don't end up mutably borrowing twice when updating
        // the current screen. This allows the screen to have a mutable reference to WordieApp when
        // it's updating.
        let mut screens = std::mem::take(&mut self.screens);

        // Update the current screen
        screens
            .last_mut()
            .expect("At least one screen must be active")
            .update(self, &ctx, frame);

        // Restore self.screens
        self.screens = screens;

        // Apply any deferred push/pop screen actions
        std::mem::take(&mut self.push_pop_actions)
            .into_iter()
            .for_each(|action| {
                match action {
                    PushPopAction::PushScreen(screen) => {
                        self.screens.push(screen);
                    },
                    PushPopAction::PopScreen => {
                        self.screens.pop();
                    },
                }
            });
    }
}

/// Main screen
#[derive(Default)]
struct MainScreen;

impl WordieAppScreen for MainScreen {
    fn update(&mut self, app: &mut WordieApp, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                WordieApp::heading(ui, "Main");

                if ui.button("Review").clicked() {
                    log::info!("Switching to review mode");
                    app.push_screen::<ReviewScreen>();
                }

                if ui.button("Add").clicked() {
                    log::info!("Switching to review mode");
                    app.push_screen::<AddScreen>();
                }
            });

            ui.label(RichText::new("Press a button instead of hanging around here")
                     .size(24.0));
        });
    }
}

/// Review screen
struct ReviewScreen {
    should_get_next_review: bool,
    cur_review: Option<Review>,
    suggested_sentences: Option<Vec<(Sentence, Vec<String>)>>,
}

impl ReviewScreen {
    fn get_next_review(&mut self, app: &mut WordieApp) {
        if self.should_get_next_review {
            log::info!("Getting next review");
            self.should_get_next_review = false;
            self.cur_review = app.srs_algorithm.get_next_card().unwrap();

            // If the next card is over our review limit, get a list of suggseted sentences too
            match self.cur_review.as_ref() {
                Some(Review::New { unknown_words, .. }) => {
                    if *unknown_words > MAX_NEW_CARDS_PER_SENTENCE {
                        self.suggested_sentences = app.srs_algorithm.get_suggested_sentences(*unknown_words).ok();
                    }
                },
                _ => {}
            }
        }
    }

    fn answer_review(&mut self, app: &mut WordieApp, difficulty: Difficulty) {
        if let Some(review) = self.cur_review.take() {
            app.srs_algorithm.review(review, difficulty).unwrap();
            self.should_get_next_review = true;
            self.cur_review = None;
        }
    }
}

impl Default for ReviewScreen {
    fn default() -> Self {
        Self {
            should_get_next_review: true,
            cur_review: None,
            suggested_sentences: None,
        }
    }
}

impl WordieAppScreen for ReviewScreen {
    fn update(&mut self, app: &mut WordieApp, ctx: &egui::Context, _: &mut eframe::Frame) {
        // Get review if there isn't a current review
        self.get_next_review(app);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;

            ui.horizontal(|ui| {
                WordieApp::heading(ui, "Review");

                if ui.button("< Back").clicked() {
                    log::info!("Leaving review mode");
                    app.pop_screen();
                }
            });

            if let Some(review) = self.cur_review.as_ref() {
                // Whether there's a card to review or not
                let show_card = match review {
                    Review::New { unknown_words, .. } => *unknown_words <= MAX_NEW_CARDS_PER_SENTENCE,
                    _ => true
                };

                // New or review card
                match (show_card, review) {
                    (false, Review::New { unknown_words, .. }) => {
                        let text = format!("No more reviews (next card is i+{}, which is greater than the limit of i+{})",
                            unknown_words, MAX_NEW_CARDS_PER_SENTENCE);
                        ui.label(RichText::new(text)
                                 .size(18.0)
                                 .color(Color32::GRAY));

                        // Show suggested sentences
                        ui.label(RichText::new(format!("Available i+{} sentences:", unknown_words))
                                 .size(18.0));

                        if let Some(suggested) = self.suggested_sentences.as_ref() {
                            for (sentence, words) in suggested.iter().take(MAX_SUGGESTED_SENTENCES) {
                                let text = format!("{} (unknown words: {})", sentence.text, words.join(", "));
                                ui.label(RichText::new(text)
                                         .size(18.0));
                            }

                        }
                        else {
                            ui.label(RichText::new("(none)")
                                     .size(18.0)
                                     .color(Color32::GRAY));
                        }
                    }
                    (true, Review::New { unknown_words, .. }) => {
                        let text = format!("New sentence (i+{unknown_words})");
                        ui.label(RichText::new(text)
                                 .size(18.0));
                    },
                    (true, Review::Due { words_due, .. }) => {
                        let text = format!("Due sentence ({words_due} words due)");
                        ui.label(RichText::new(text)
                                 .size(18.0));
                    },
                    _ => { panic!("This should never happen") }
                }

                if show_card {
                    // Sentence text
                    let review_text = format!("{}", review.sentence().text);
                    ui.label(RichText::new(review_text)
                             .color(Color32::WHITE)
                             .size(28.0));

                    // Answer buttons
                    ui.horizontal(|ui| {
                        for difficulty in Difficulty::iter() {
                            if ui.button(format!("{difficulty:?}")).clicked() {
                                self.answer_review(app, difficulty);
                            }
                        }
                    });
                }
            }
            else {
                ui.label(RichText::new("No more reviews")
                         .size(18.0)
                         .color(Color32::GRAY));
            }

            // Review stats
            let review_stats = format!("{} cards learned today, {} cards reviewed today",
                                       app.srs_algorithm.cards_learned_today(),
                                       app.srs_algorithm.cards_reviewed_today());

            ui.label(RichText::new(review_stats).size(18.0));
        });
    }
}

/// Add screen
struct AddScreen {
    input_text: String,
    status_text: Option<String>,
}

impl Default for AddScreen {
    fn default() -> Self {
        Self {
            input_text: String::new(),
            status_text: None,
        }
    }
}

impl WordieAppScreen for AddScreen {
    fn update(&mut self, app: &mut WordieApp, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                WordieApp::heading(ui, "Add");

                if ui.button("< Back").clicked() {
                    log::info!("Leaving add mode");
                    app.pop_screen();
                }
            });

            for file in ctx.input().raw.dropped_files.iter() {
                log::info!("Got dropped file: {file:?}");
                if let Some(path) = file.path.as_ref() {
                    if let Ok(text) = std::fs::read_to_string(path) {
                        self.input_text = to_sentences(text.as_str()).join("\n");
                    }
                    else {
                        self.status_text = Some(format!("Invalid file {path:?}"));
                    }
                }
            }

            let available_size = ui.available_size();

            let button_size = egui::Vec2::new(available_size.x, 20.0);
            let status_text_size = match self.status_text {
                Some(_) => egui::Vec2::new(available_size.x, 20.0),
                _ => egui::Vec2::new(0.0, 0.0),
            };
            let text_edit_size = egui::Vec2::new(available_size.x, available_size.y - button_size.y - status_text_size.x);

            egui::ScrollArea::new([false, true]).max_height(text_edit_size.y).show(ui, |ui| {
                ui.add_sized(text_edit_size, egui::TextEdit::multiline(&mut self.input_text).desired_rows(10).desired_width(text_edit_size.x));
            });

            if ui.add_sized(button_size, egui::Button::new("Add sentences (one per line)")).clicked() {
                log::info!("Adding sentences");

                let sentences = self.input_text
                    .lines()
                    .map(|line| Sentence {
                        id: uuid::Uuid::new_v4(),
                        text: line.to_owned(),
                    })
                    .collect::<Vec<Sentence>>();

                let result = app.srs_algorithm.add_sentences(&sentences);

                if let Err(err) = result {
                    self.status_text = Some(err.to_string());
                }
                else {
                    self.input_text.clear();
                }
            }

            if let Some(status_text) = self.status_text.as_ref() {
                let text = RichText::new(status_text).color(Color32::LIGHT_RED);
                ui.add_sized(status_text_size, egui::Label::new(text));
            }
        });
    }
}

fn to_sentences(s: &str) -> Vec<String> {
    let terminators: HashSet<char> = HashSet::from(['。', '\n']);
    let open_quotes: HashSet<char> = HashSet::from(['「']);
    let close_quotes: HashSet<char> = HashSet::from(['」']);
    let ambiguous_quotes: HashSet<char> = HashSet::from(['\'', '"']);

    let mut result = Vec::new();

    let mut depth: i32 = 0;
    let mut cur_string: String = String::new();
    for c in s.chars() {
        cur_string.push(c);

        if open_quotes.contains(&c) {
            depth += 1;
        }
        else if close_quotes.contains(&c) {
            depth -= 1;
        }
        else if ambiguous_quotes.contains(&c) {
            // Don't allow nested quotes like this.. Just assume if we're in a quote already to
            // leave it.
            if depth > 0 {
                depth -= 1;
            }
            else {
                depth += 1;
            }
        }
        else if depth == 0 && terminators.contains(&c) {
            let sentence = cur_string.trim();

            if !sentence.is_empty() {
                result.push(sentence.to_string());
            }

            cur_string.clear();
        }
    }

    result
}
