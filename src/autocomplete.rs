use std::{ops::Range, ptr::NonNull, sync::Arc};

use byteyarn::yarn;
use egui::{text::{CCursor, LayoutJob}, text_selection::CCursorRange, Align, Color32, FontSelection, Sense, Ui, Vec2};
use vince621_core::{db::tags::{Tag, TagAndImplicationDatabase, TagCategory}, search::e6_posts::parse_query_for_autocomplete};

const MAX_AUTOCOMPLETION_COUNT: usize = 20;

pub struct Autocompleter{
    tag_db: Arc<TagAndImplicationDatabase>,
    last_result: Option<AutocompleteResult>,
}

fn ptr_diff<T:?Sized>(p1: *const T, p2: *const T) -> usize {
    p2.addr() - p1.addr()
}

struct AutocompleteResult {
    // SAFETY: The "lifetime" of these pointers is the lifetime of the Arc.  It relies on the
    // contents of the Arc not moving and the contents of all of the boxed slices inside that Arc
    // not moving, which I think should be guaranteed by Arc immutability.
    matches: Vec<(NonNull<Tag>, Option<NonNull<str>>)>,
    token_range: Range<usize>,
}

impl Autocompleter {
    pub fn do_autocomplete(&mut self, search_query: &str, char_index: usize) -> bool {
        let byte_index = search_query.char_indices().nth(char_index).map(|x| x.0).unwrap_or(search_query.len());
        if let Some((token, ancestors)) = parse_query_for_autocomplete(search_query, byte_index) {
            let token_start_byte_offset = (token.as_ptr() as usize - search_query.as_ptr() as usize);
            let cursor_idx_in_token = byte_index - token_start_byte_offset;
            let (prefix, suffix) = token.split_at(cursor_idx_in_token);
            let ancestors = ancestors.into_iter().filter_map(|token| self.tag_db.get(token)).map(|x|x.id).collect::<Vec<_>>();
            let matches = self.tag_db.autocomplete(prefix, MAX_AUTOCOMPLETION_COUNT, |tag, alias| {
                !ancestors.contains(&tag.id) && alias.unwrap_or(tag.name.as_str()).ends_with(suffix)
            });
            let matches = matches.into_iter().map(|(a,b)| (a.into(), b.map(Into::into))).collect::<Vec<(NonNull<Tag>, Option<NonNull<str>>)>>();
            let range_start = ptr_diff(search_query, token);
            let token_range = range_start..range_start+token.len();
            self.last_result = Some(AutocompleteResult{matches, token_range});
            true
        } else {
            self.last_result = None;
            false
        }
    }

    pub fn show_autocomplete_ui(&self, search_query: &mut String, ui: &mut Ui) -> Option<CCursorRange> {
        let AutocompleteResult{token_range, matches} = self.last_result.as_ref().expect("do_autocomplete() should have been called first");

        for (tag_ptr, alias_ptr) in matches {
            let tag;
            let alias;
            unsafe {
                // SAFETY: The "lifetime" of these pointers is the lifetime of self.tag_db.  These pointers being valid relies on the
                // contents of the Arc not moving and the contents of all of the boxed slices inside that Arc not moving, which I think 
                // should be guaranteed by Arc immutability.
                //
                // If at some point in the future I discover a way that those boxes might move, I'll panic, but for now I think I'm good.
                tag = tag_ptr.as_ref();
                alias = alias_ptr.map(|x|x.as_ref());
            }
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
                search_query.replace_range(token_range.clone(), tag.name.as_str());
                let mut end_pos = token_range.start + tag.name.len();
                if end_pos == search_query.len() {
                    search_query.push(' ');
                    end_pos += 1;
                }

                let end = CCursor::new(search_query[..end_pos].chars().count());

                return Some(CCursorRange::one(end));
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

        None

    }

    pub(crate) fn new(tag_db: Arc<TagAndImplicationDatabase>) -> Autocompleter {
        Self {tag_db, last_result: None}
    }
}
