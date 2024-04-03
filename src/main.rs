use std::{sync::{Arc, Mutex}, time::Instant};

use eframe::NativeOptions;
use eframe::egui::{self, CentralPanel, Key, ProgressBar, TextEdit, TopBottomPanel};
use egui::load::BytesPoll;
use rayon::{iter::{IndexedParallelIterator as _, ParallelIterator as _}, slice::ParallelSliceMut as _};
use vince621_core::{db::{posts::PostDatabase, tags::TagDatabase}, search::e6_posts::SortOrder};


use paste::paste;

enum UiState {
    ShowText(String),
    Searching(rayon_progress::ItemsProcessed, usize),
    ShowPosts(Vec<usize>, usize),
}

struct App {
    search_query: String,
    ui_state: Arc<Mutex<UiState>>,
    tag_db: TagDatabase,
    post_db: Arc<PostDatabase>,
}
impl App {
    fn new(ctx: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&ctx.egui_ctx);
        let tag_db = vince621_csv::tags::load_tag_database(csv::Reader::from_reader(flate2::read::GzDecoder::new(std::fs::File::open("tags.csv.gz").unwrap()))).unwrap();
        let post_db = vince621_serialization::posts::deserialize_post_database(&mut std::io::BufReader::new(std::fs::File::open("posts.v621").unwrap())).unwrap();
        Self {
            search_query: String::new(),
            ui_state: Arc::new(Mutex::new(UiState::ShowText("Enter a search query".into()))),
            tag_db,
            post_db: Arc::new(post_db),
        }
    }

    fn start_search(&self) {
        let state = self.ui_state.clone();
        let post_db = self.post_db.clone();
        let (query, sort_order) = match vince621_core::search::e6_posts::parse_query_and_sort_order(&self.tag_db, &self.search_query) {
            Ok(x) => x,
            Err(e) => {
                *state.lock().unwrap() = UiState::ShowText(e.cause().unwrap().to_string());
                return;
            }
        };
        rayon::spawn(move || {
            let searcher = rayon_progress::ProgressAdaptor::new(post_db.get_all());
            *state.lock().unwrap() = UiState::Searching(searcher.items_processed(), searcher.len());
            let t1 = Instant::now();
            let mut results = searcher.enumerate().filter(|(_, post)| query.validate(post)).map(|(idx, _)| idx).collect::<Vec<_>>();
            let elapsed = t1.elapsed();
            println!("search took {:?}", elapsed);
            let t2 = Instant::now();
            match sort_order {
                SortOrder::DateAscending => {
                    // post database is already sorted by that -- we don't need to do anything
                },
                SortOrder::Date => {
                    results.reverse();
                },
                SortOrder::Random => {
                    todo!()
                }
                other => {
                    let posts = post_db.get_all();
                    macro_rules! match_arms {
                        ($match_on:ident, $results: ident, $posts: ident, $($order: ident => post.$field:ident),*) => {
                            paste!(
                            match $match_on {
                                $(
                                    SortOrder::$order => $results.par_sort_unstable_by(|a,b| $posts[*b].$field.cmp(&$posts[*a].$field)),
                                    SortOrder::[<$order Ascending>] => $results.par_sort_unstable_by(|a,b| $posts[*a].$field.cmp(&posts[*b].$field)),
                                )*
                                SortOrder::Date | SortOrder::DateAscending | SortOrder::Random => unreachable!()
                            }
                            )
                        }
                    }
                    match_arms!(other, results, posts, Score => post.score, FavCount => post.fav_count);

                },
            }
            println!("sort took {:?}", t2.elapsed());
            *state.lock().unwrap() = UiState::ShowPosts(results, 0);
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        TopBottomPanel::top("search").show(ctx, |ui| ui.horizontal(|ui| {
            let resp = ui.add(TextEdit::singleline(&mut self.search_query));
            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) { //eewwwwww
                self.start_search();
            }
            if ui.button("Search").clicked() {
                self.start_search();
            }
        }));

        CentralPanel::default().show(ctx, |ui| {
            match *self.ui_state.lock().unwrap() {
                UiState::ShowText(ref s) => {
                    ui.centered_and_justified(|ui| ui.label(s));
                },
                UiState::Searching(ref cur, max) => {
                    let progress = (cur.get() as f32) / (max as f32);
                    ui.centered_and_justified(|ui| ui.add(ProgressBar::new(progress).show_percentage()));
                    ctx.request_repaint();
                },
                UiState::ShowPosts(ref results, ref mut idx) => {
                    if results.is_empty() {
                        ui.label("No results");
                    } else {
                        if !ctx.wants_keyboard_input() {
                            if ui.input(|i| i.key_pressed(Key::ArrowLeft)) && *idx > 0 {
                                *idx -= 1;
                            } else if ui.input(|i| i.key_pressed(Key::ArrowRight)) && *idx < results.len()-1 {
                                *idx += 1;
                            }
                        }
                        let post_idx = results[*idx];
                        let posts = self.post_db.get_all();
                        ui.label(format!("Showing result {} of {} (id {})", *idx+1, results.len(), posts[post_idx].id));
                        ui.image(posts[post_idx].url());

                        // preload the next couple images so they display faster.
                        let next_idx = (*idx+1).min(results.len()-1);
                        let last_idx = (*idx+5).min(results.len()-1);
                        for post_idx in results[next_idx..last_idx].iter() {
                            let _ = ctx.try_load_bytes(&posts[*post_idx].url());
                        }
                    }
                },
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    eframe::run_native("vince621", NativeOptions {
        ..NativeOptions::default()
    }, Box::new(|ctx| Box::new(App::new(ctx))))
}
