use std::{sync::{Arc, Mutex}, time::Instant};

use directories::ProjectDirs;
use eframe::{egui_wgpu::WgpuConfiguration, wgpu::{self, PowerPreference}};
use eframe::NativeOptions;
use eframe::egui::{self, CentralPanel, Key, ProgressBar, TextEdit, TopBottomPanel};
use egui::{popup_below_widget, text::LayoutJob, Align, Color32, FontSelection, Id, Layout, Rect, RichText, Sense, Vec2, WidgetText};
use rand::seq::SliceRandom as _;
use rayon::{iter::{IndexedParallelIterator as _, ParallelIterator as _}, slice::ParallelSliceMut as _};
use vince621_core::{db::{posts::{ImageResolution, PostDatabase}, tags::{TagCategory, TagDatabase}}, search::e6_posts::SortOrder};

use paste::paste;

//mod image_loader;

//use image_loader::loader::ImageLoader;

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
    fn new(ctx: &eframe::CreationContext<'_>, tag_db: TagDatabase, post_db: PostDatabase) -> Self {
        egui_extras::install_image_loaders(&ctx.egui_ctx);
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
                    results.shuffle(&mut rand::thread_rng());
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
            if resp.has_focus() && !self.search_query.ends_with('}') {
                let pos = self.search_query.rfind([' ','{']).map(|x|x+1).unwrap_or(0);
                let last_token = &self.search_query[pos..];
                let matches = self.tag_db.autocomplete(last_token, 20);

                ui.memory_mut(|mem| mem.open_popup(Id::new("tag_autocomplete_dropdown")));

                popup_below_widget(&ui, Id::new("tag_autocomplete_dropdown"), &resp, |ui| {
                    for tag in matches {
                        // egui does not natively support putting two text fields on the same row,
                        // so we have to manually implement a custom widget.
                        let color = match tag.category {
                            TagCategory::General => Color32::from_rgb(0xb4,0xc7,0xd9),
                            TagCategory::Artist => Color32::from_rgb(0xf2,0xac,0x08),
                            TagCategory::Copyright => Color32::from_rgb(0xdd,0x00,0xdd),
                            TagCategory::Character => Color32::from_rgb(0x00,0xaa,0x00),
                            TagCategory::Species => Color32::from_rgb(0xed,0x5d,0x1f),
                            TagCategory::Invalid => Color32::from_rgb(0xff,0x3d,0x3d),
                            TagCategory::Meta => Color32::from_rgb(0xff,0xff,0xff),
                            TagCategory::Lore => Color32::from_rgb(0x22,0x88,0x22),
                        };
                        let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 20.0), Sense::click());
                        
                        if response.hovered() {
                            painter.rect_filled(response.rect, ui.style().visuals.menu_rounding, ui.style().visuals.extreme_bg_color);
                        }

                        let font = FontSelection::Default.resolve(ui.style());
                        let name_galley = ui.fonts(|fonts| fonts.layout_no_wrap(tag.name.to_string(), font.clone(), color));

                        let mut post_count_job = LayoutJob::simple_singleline(tag.post_count.to_string(), font, color);
                        post_count_job.halign=Align::RIGHT;
                        let post_count_galley = ui.fonts(|fonts| fonts.layout_job(post_count_job));

                        painter.galley(response.rect.left_top(), name_galley, Color32::WHITE);
                        painter.galley(response.rect.right_top(), post_count_galley, Color32::WHITE);

                    }
                });
            } else if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) { //eewwwwww
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
                        ui.image(posts[post_idx].url(ImageResolution::Sample));

                        // preload the next couple images so they display faster.
                        let next_idx = (*idx+1).min(results.len());
                        let last_idx = (*idx+5).min(results.len());
                        for post_idx in results[next_idx..last_idx].iter() {
                            let _ = ctx.try_load_bytes(&posts[*post_idx].url(ImageResolution::Sample));
                        }
                    }
                },
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    let Some(proj_dirs) = ProjectDirs::from("blue", "spacestation", "vince621") else {
        println!("Couldn't decide where to put config directories!");
        return Ok(())
    };
    let (tag_db, post_db) = rayon::join(
        || vince621_serialization::deserialize_tag_database(&mut std::io::BufReader::new(std::fs::File::open(proj_dirs.cache_dir().join("tags.v621")).unwrap())).unwrap(),
        || vince621_serialization::deserialize_post_database(&mut std::io::BufReader::new(std::fs::File::open(proj_dirs.cache_dir().join("posts.v621")).unwrap())).unwrap()
    );
    let image_dir = proj_dirs.cache_dir().join("images");
    match std::fs::create_dir(&image_dir) {
        Ok(()) => {
            println!("created cache/images/ directory");
        }
        Err(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
        Err(e) => panic!("Error creating cache/imgaes/ directory: {}", e),
    }
    eframe::run_native("vince621", NativeOptions {
        wgpu_options: WgpuConfiguration {
            // default to the low power GPU -- we're not doing anything graphically fancy
            power_preference: wgpu::util::power_preference_from_env().unwrap_or(PowerPreference::LowPower),
            // ensure our application has access to the full GPU limits the hardware has.
            // by default, eframe restricts us to a max texture size of 8192x8192, and many of the
            // images on e621 are... larger than that.  and I'd rather not worry about carving a
            // single image into multiple textures until I *have* to.
            device_descriptor: Arc::new(|adapter| wgpu::DeviceDescriptor{
                label: Some("egui wgpu device"),
                required_features: wgpu::Features::default(),
                required_limits: adapter.limits(),
            }),
            ..WgpuConfiguration::default()
        },
        ..NativeOptions::default()
    }, Box::new(|ctx| {
        /*
        ctx.egui_ctx.data_mut(|data| {
            data.insert_temp(Id::NULL, Arc::new(ImageLoader::new(image_dir)));
        });
        */
        Box::new(App::new(ctx, tag_db, post_db))
    }))
}
