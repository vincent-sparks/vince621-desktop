use std::{sync::{Arc, Mutex, MutexGuard}, time::Instant};

use directories::ProjectDirs;
use eframe::{egui_wgpu::WgpuConfiguration, wgpu::{self, PowerPreference}};
use eframe::NativeOptions;
use eframe::egui::{self, CentralPanel, Key, ProgressBar, TextEdit, TopBottomPanel};
use egui::{load::BytesPoll, popup_below_widget, text::{CCursor, LayoutJob}, text_selection::CCursorRange, Align, Color32, FontSelection, Id, Layout, Rect, RichText, Sense, Ui, Vec2, ViewportBuilder, WidgetText};
use rand::seq::SliceRandom as _;
use rayon::{iter::{IndexedParallelIterator as _, ParallelIterator as _}, slice::ParallelSliceMut as _};
use ruffle_core::{tag_utils::SwfMovie, PlayerBuilder};
use vince621_core::{db::{posts::{FileExtension, ImageResolution, PostDatabase}, tags::{TagAndImplicationDatabase, TagCategory}}, search::{e6_posts::{PostKernel, SortOrder}, NestedQuery}};

use byteyarn::yarn;

use paste::paste;

use egui_ruffle::{Descriptors, EguiRufflePlayer};

/*
mod image_loader;

use image_loader::loader::ImageLoader;
*/

mod ruffle_util;

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
    tag_db: TagAndImplicationDatabase,
    post_db: Arc<PostDatabase>,
    settings: Arc<Mutex<Settings>>,
    ruffle_descriptors: Arc<Descriptors>,
    flashplayer: Option<EguiRufflePlayer>,
}

impl App {
    fn new(ctx: &eframe::CreationContext<'_>, tag_db: TagAndImplicationDatabase, post_db: PostDatabase) -> Self {
        egui_extras::install_image_loaders(&ctx.egui_ctx);
        Self {
            search_query: String::new(),
            ui_state: Arc::new(Mutex::new(UiState::ShowText("Enter a search query".into()))),
            tag_db,
            post_db: Arc::new(post_db),
            settings: Arc::new(Mutex::new(Settings::default())),
            flashplayer: None,
            ruffle_descriptors: Arc::new(egui_ruffle::create_descriptors_from_render_state(ctx.wgpu_render_state.as_ref().expect("flash support requires wgpu"))),
        }
    }

    fn start_search(&self) -> Option<(usize,usize)> {
        let state = self.ui_state.clone();
        let post_db = self.post_db.clone();
        let (query, sort_order) = match vince621_core::search::e6_posts::parse_query_and_sort_order(&self.tag_db.tags, &self.search_query) {
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
            if textbox.response.gained_focus() {//&& !self.search_query.ends_with('}') && !self.search_query.ends_with(' ') {
                ui.memory_mut(|mem| mem.open_popup(Id::new("tag_autocomplete_dropdown")));

            } else if textbox.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) { //eewwwwww
                error_range = self.start_search();
            }
            if ui.button("Search").clicked() {
                error_range = self.start_search();
            }

            if let Some((start, end)) = error_range {
                textbox.state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(start), CCursor::new(end))));
                textbox.state.clone().store(ui.ctx(),textbox.response.id);
                textbox.response.request_focus();
            }
            popup_below_widget(&ui, Id::new("tag_autocomplete_dropdown"), &textbox.response, |ui| {
                let mut pos = self.search_query.rfind([' ','{']).map(|x|x+1).unwrap_or(0);
                if pos < self.search_query.len() {
                    let c = self.search_query.as_bytes()[pos];
                    if c == b'-' || c == b'~' {
                        pos += 1;
                    }
                }

                let last_token = &self.search_query[pos..];
                let matches = self.tag_db.autocomplete(last_token, 20);

                for (tag, alias) in matches {
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

                    let tag_name = match alias {
                        Some(alias) => {
                            yarn!("{} -> {}", alias, tag.name)
                        },
                        None => tag.name.aliased(),
                    };
                    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 20.0), Sense::click());
                    
                    if response.hovered() || response.has_focus() {
                        painter.rect_filled(response.rect, ui.style().visuals.menu_rounding, ui.style().visuals.extreme_bg_color);
                    }

                    if response.clicked() {
                        self.search_query.truncate(pos);
                        self.search_query.push_str(tag.name.as_str());
                        self.search_query.push(' ');

                        let end = CCursor::new(self.search_query.chars().count());

                        textbox.state.cursor.set_char_range(Some(CCursorRange::one(end)));
                        textbox.state.store(ui.ctx(),textbox.response.id);
                        textbox.response.request_focus();
                        ui.memory_mut(|mem| mem.close_popup());
                        break;
                    }

                    let font = FontSelection::Default.resolve(ui.style());
                    let name_galley = ui.fonts(|fonts| fonts.layout_no_wrap(tag_name.to_string(), font.clone(), color));

                    let mut post_count_job = LayoutJob::simple_singleline(tag.post_count.to_string(), font, color);
                    post_count_job.halign=Align::RIGHT;
                    let post_count_galley = ui.fonts(|fonts| fonts.layout_job(post_count_job));

                    let widget_rect = response.rect.shrink2(ui.style().spacing.button_padding);

                    let name_top_offset = (widget_rect.height() - name_galley.rect.height()) / 2.0;
                    let post_count_top_offset = (widget_rect.height() - post_count_galley.rect.height()) / 2.0;

                    painter.galley(widget_rect.left_top() + Vec2::new(0.0, name_top_offset), name_galley, Color32::WHITE);
                    painter.galley(widget_rect.right_top() + Vec2::new(0.0, post_count_top_offset), post_count_galley, Color32::WHITE);

                }
            });
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
                                            self.flashplayer = Some(EguiRufflePlayer::new(PlayerBuilder::new().with_movie(movie), frame.wgpu_render_state().expect("flashplayer requires wgpu"), self.ruffle_descriptors.clone(), (1,1)).expect("Could not create flashplayer"));
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
            data.insert_temp(Id::NULL, Arc::new(ImageLoader::new(ctx.egui_ctx.clone(), image_dir)));
        });
        */
        Box::new(App::new(ctx, tag_db, post_db))
    }))
}
