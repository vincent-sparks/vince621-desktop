#![feature(strict_provenance)]
use std::{sync::{Arc, Mutex, MutexGuard}, time::Instant};

use directories::ProjectDirs;
use eframe::{egui_wgpu::WgpuConfiguration, wgpu::{self, PowerPreference}};
use eframe::NativeOptions;
use eframe::egui::{self, CentralPanel, Key, ProgressBar, TextEdit, TopBottomPanel};
use egui::{load::BytesPoll, popup_below_widget, text::{CCursor, LayoutJob}, text_selection::CCursorRange, Align, Color32, FontSelection, Id, Layout, Rect, RichText, Sense, Ui, Vec2, ViewportBuilder, WidgetText};
use http_body_util::Empty;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use rand::seq::SliceRandom as _;
use rayon::{iter::{IndexedParallelIterator as _, ParallelIterator as _}, slice::ParallelSliceMut as _};
use ruffle_core::{tag_utils::SwfMovie, PlayerBuilder};
use vince621_core::{db::{posts::{FileExtension, ImageResolution, PostDatabase}, tags::{TagAndImplicationDatabase, TagCategory}}, search::{e6_posts::{parse_query_for_autocomplete, PostKernel, SortOrder}, NestedQuery}};

use byteyarn::yarn;

use paste::paste;

use egui_ruffle::{Descriptors, EguiRufflePlayer};

/*
mod image_loader;

use image_loader::loader::ImageLoader;
*/

mod ruffle_util;
use ruffle_util::storage::DiskStorageBackend;

mod autocomplete;
use autocomplete::Autocompleter;

mod db_download;

type Client = hyper_util::client::legacy::Client<HttpsConnector<HttpConnector>,Empty<&'static [u8]>>;

enum UiState {
    ShowText(String),
    Searching(rayon_progress::ItemsProcessed, usize),
    ShowPosts(Vec<usize>, usize),
}

#[derive(Default)]
struct Settings {
    settings_dialog_is_open: bool,
    user_blacklist: Vec<NestedQuery<PostKernel>>,
}

struct App {
    search_query: String,
    ui_state: Arc<Mutex<UiState>>,
    tag_db: Arc<TagAndImplicationDatabase>,
    post_db: Arc<PostDatabase>,
    settings: Arc<Mutex<Settings>>,
    ruffle_descriptors: Arc<Descriptors>,
    flashplayer: Option<EguiRufflePlayer>,
    project_dirs: ProjectDirs,
    autocompleter: Autocompleter,
}

impl App {
    fn new(ctx: &eframe::CreationContext<'_>, tag_db: TagAndImplicationDatabase, post_db: PostDatabase, project_dirs: ProjectDirs) -> Self {
        let tag_db = Arc::new(tag_db);
        egui_extras::install_image_loaders(&ctx.egui_ctx);
        Self {
            search_query: String::new(),
            autocompleter: Autocompleter::new(tag_db.clone()),
            ui_state: Arc::new(Mutex::new(UiState::ShowText("Enter a search query".into()))),
            tag_db,
            post_db: Arc::new(post_db),
            settings: Arc::new(Mutex::new(Settings::default())),
            flashplayer: None,
            ruffle_descriptors: Arc::new(egui_ruffle::create_descriptors_from_render_state(ctx.wgpu_render_state.as_ref().expect("flash support requires wgpu"))),
            project_dirs,
        }
    }

    fn start_search(&self) -> Option<(usize,usize)> {
        let state = self.ui_state.clone();
        let post_db = self.post_db.clone();
        let parse_tag_fn = |s| self.tag_db.search_wildcard(s).map(|tag| tag.id).collect::<Vec<u32>>();
        let (query, sort_order) = match vince621_core::search::e6_posts::parse_query_and_sort_order(parse_tag_fn, &self.search_query) {
            Ok(x) => x,
            Err(e) => {
                let (start_pos, end_pos) = e.get_range(&self.search_query);
                // cursor positions expect character offsets, not byte offsets, so we need to
                // convert them.
                let start_pos = self.search_query[..start_pos].chars().count();
                let end_pos = self.search_query[..end_pos].chars().count();

                *state.lock().unwrap() = UiState::ShowText(e.into_reason());
                return Some((start_pos, end_pos));
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
        None
    }

    fn show_settings_dialog(mut settings: MutexGuard<'_, Settings>, ui: &mut Ui) {
        if ui.input(|i| i.viewport().close_requested()) {
            settings.settings_dialog_is_open=false;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        {
            let mut settings = self.settings.lock().unwrap();
            TopBottomPanel::top("menu").show(ctx, |ui| egui::menu::bar(ui, |ui| {
                ui.menu_button("Settings", |ui| {
                    if ui.button("Settings").clicked() {
                        settings.settings_dialog_is_open=true;
                    }
                });
            }));
            if settings.settings_dialog_is_open {
                let settings = self.settings.clone();
                ctx.show_viewport_deferred(egui::ViewportId(Id::new("settings_dialog")),
                ViewportBuilder::default(),
                move |ctx, _class| {
                    CentralPanel::default().show(ctx, |ui| {
                        App::show_settings_dialog(settings.lock().unwrap(), ui);
                    });
                });
            }
        }
        TopBottomPanel::top("search").show(ctx, |ui| ui.horizontal(|mut ui| {
            let mut textbox = TextEdit::singleline(&mut self.search_query).show(&mut ui);
            let mut error_range = None;
            if textbox.response.has_focus() {//&& !self.search_query.ends_with('}') && !self.search_query.ends_with(' ') {
                // TODO predicate this also on whether the text was modified and/or the cursor moved.
                if let Some(pos) = textbox.state.cursor.char_range() {
                    if textbox.response.changed() {
                        if self.autocompleter.do_autocomplete(&self.search_query, pos.primary.index) {
                            ui.memory_mut(|mem| mem.open_popup(Id::new("tag_autocomplete_dropdown")));
                        } else {
                            ui.memory_mut(|mem| mem.close_popup());
                        }
                    }
                } else {
                    ui.memory_mut(|mem| mem.close_popup());
                }

            } else if textbox.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) { //eewwwwww
                self.flashplayer=None;
                error_range = self.start_search();
            }
            if ui.button("Search").clicked() {
                self.flashplayer=None;
                error_range = self.start_search();
            }

            let range = if let Some((start, end)) = error_range {
                Some(CCursorRange::two(CCursor::new(start), CCursor::new(end)))
            } else {
                popup_below_widget(&ui, Id::new("tag_autocomplete_dropdown"), &textbox.response, |ui| self.autocompleter.show_autocomplete_ui(&mut self.search_query, ui)).flatten()
            };

            if let Some(range) = range {
                textbox.state.cursor.set_char_range(Some(range));
                textbox.state.clone().store(ui.ctx(),textbox.response.id);
                textbox.response.request_focus();
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
                                self.flashplayer=None;
                            } else if ui.input(|i| i.key_pressed(Key::ArrowRight)) && *idx < results.len()-1 {
                                *idx += 1;
                                self.flashplayer=None;
                            }
                        }
                        let post_idx = results[*idx];
                        let posts = self.post_db.get_all();
                        let post = &posts[post_idx];
                        ui.label(format!("Showing result {} of {} (id {})", *idx+1, results.len(), post.id));
                        
                        match post.file_ext {
                            FileExtension::SWF => {
                                if let Some(player) = self.flashplayer.as_mut() {
                                    player.show(ui);
                                } else {
                                    match ctx.try_load_bytes(&post.url(ImageResolution::Full)) {
                                        Ok(BytesPoll::Pending { .. }) => {
                                            ui.spinner();
                                        },
                                        Ok(BytesPoll::Ready { bytes, .. }) => {
                                            let movie = SwfMovie::from_data(&bytes, post.url(ImageResolution::Full), None).expect("error loading movie");
                                            let builder = PlayerBuilder::new()
                                                .with_movie(movie)
                                                .with_storage(Box::new(DiskStorageBackend::new(self
                                                                                               .project_dirs
                                                                                               .data_dir()
                                                                                               .join("flash_saves")
                                                                                               .join(post.id.get().to_string())
                                                                                               )))
                                                .with_video(ruffle_video_software::backend::SoftwareVideoBackend::new())
                                                ;
                                            self.flashplayer = Some(EguiRufflePlayer::new(builder, frame.wgpu_render_state().expect("flashplayer requires wgpu"), self.ruffle_descriptors.clone(), (1,1)).expect("Could not create flashplayer"));
                                        },
                                        Err(_) => {
                                        },
                                    }
                                }
                            },
                            _ => {
                                ui.image(post.url(ImageResolution::Sample));
                            },
                        }

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
    let Some(proj_dirs) = ProjectDirs::from("blue", "spacestation", "Vince621") else {
        println!("Couldn't decide where to put config directories!");
        return Ok(())
    };
    let (tag_db, post_db) = rayon::join(
        || {
            let mut f = std::io::BufReader::new(std::fs::File::open(proj_dirs.cache_dir().join("tags.v621")).expect("Error opening tags file"));
            let hdr = vince621_serialization::tags::read_tag_header(&mut f).expect("error reading tag header");
            vince621_serialization::tags::deserialize_tag_and_implication_database(hdr, &mut f).unwrap()
        },
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
            device_descriptor: Arc::new(|adapter| {
                let mut features = Default::default();
                let try_features = [
                    wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                    wgpu::Features::SHADER_UNUSED_VERTEX_OUTPUT,
                    wgpu::Features::TEXTURE_COMPRESSION_BC,
                    wgpu::Features::FLOAT32_FILTERABLE,
                ];

                for feature in try_features {
                    if adapter.features().contains(feature) {
                        features |= feature;
                    }
                }
                wgpu::DeviceDescriptor{
                    label: Some("egui wgpu device"),
                    required_features: features,
                    required_limits: adapter.limits(),
                }
            }),
            ..WgpuConfiguration::default()
        },
        ..NativeOptions::default()
    }, Box::new(|ctx| {
        /*
        ctx.egui_ctx.data_mut(|data| {
            data.insert_temp(Id::NULL, Arc::new(ImageLoader::new(ctx.egui_ctx.clone(), image_dir)));
        });
        */
        Box::new(App::new(ctx, tag_db, post_db, proj_dirs))
    }))
}
