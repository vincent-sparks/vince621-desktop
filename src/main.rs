use std::sync::{Arc, Mutex};

use eframe::NativeOptions;
use egui::{CentralPanel, Key, ProgressBar, TextEdit, TopBottomPanel};


enum UiState {
    Waiting,
    Searching(rayon_progress::ItemsProcessed, usize),
    Complete(Vec<usize>),
}

struct App {
    search_query: String,
    ui_state: Arc<Mutex<UiState>>,
}
impl App {
    fn new(ctx: &eframe::CreationContext<'_>) -> Self {
        Self {
            search_query: String::new(),
            ui_state: Arc::new(Mutex::new(UiState::Waiting)),
        }
    }

    fn start_search(&self) {
        println!("I'd put the search code here IF I HAD ANY");
        /*
        let state = self.ui_state.clone();
        let query = vince621_core::search::e6_posts::parse_query(&self.tag_db, &self.search_query);
        */
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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
                UiState::Waiting => ui.centered_and_justified(|ui| ui.label("Enter a search query")),
                UiState::Searching(ref cur, max) => {
                    let progress = (cur.get() as f32) / (max as f32);
                    ui.centered_and_justified(|ui| ui.add(ProgressBar::new(progress).show_percentage()))
                },
                UiState::Complete(ref _results) => todo!(),
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    eframe::run_native("vince621", NativeOptions {
        ..NativeOptions::default()
    }, Box::new(|ctx| Box::new(App::new(ctx))))
}
