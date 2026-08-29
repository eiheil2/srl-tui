//! Main application state and logic.

use std::time::Instant;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{block::BorderType, Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::theme::Theme;
use super::widgets::{
    CompletionScreen, FlashcardWidget, HeaderBar, KeyHints, RatingButtons, StatsBar,
};
use crate::config::Config;
use crate::models::{Deck, ReviewRating};
use crate::sm2::Scheduler;
use crate::storage::{DeckInfo, DeckStorage};

// ══════════════════════════════════════════════════════════════════════════
// Application State
// ══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    DeckSelect,
    Study,
    AddCard,
    CardBrowser,
    Stats,
    Complete,
}

/// Inline text-input dialog on the deck select screen (create / rename /
/// backup import path).
#[derive(Debug, Clone, PartialEq)]
pub enum DeckInput {
    Create { name: String },
    Rename { deck_id: String, name: String },
    ImportBackup { path: String },
}

impl DeckInput {
    fn value(&self) -> &str {
        match self {
            DeckInput::Create { name }
            | DeckInput::Rename { name, .. }
            | DeckInput::ImportBackup { path: name } => name,
        }
    }

    fn value_mut(&mut self) -> &mut String {
        match self {
            DeckInput::Create { name }
            | DeckInput::Rename { name, .. }
            | DeckInput::ImportBackup { path: name } => name,
        }
    }

    fn title(&self) -> &'static str {
        match self {
            DeckInput::Create { .. } => " New Deck ",
            DeckInput::Rename { .. } => " Rename Deck ",
            DeckInput::ImportBackup { .. } => " Import Backup (path) ",
        }
    }
}

pub struct App {
    pub screen: Screen,
    pub running: bool,

    // Config and theme
    pub config: Config,
    pub theme: Theme,

    // Storage
    pub storage: DeckStorage,
    pub scheduler: Scheduler,

    // Deck selection
    pub deck_list: Vec<DeckInfo>,
    pub deck_list_state: ListState,
    pub deck_delete_pending: bool,
    pub deck_input: Option<DeckInput>,

    // Current deck
    pub current_deck: Option<Deck>,

    // Study state
    pub study_queue: Vec<usize>, // Indices into deck.cards
    pub current_card_idx: Option<usize>,
    pub showing_answer: bool,
    pub answer_revealed: bool, // True once answer has been shown at least once
    pub cards_studied: usize,
    pub session_start: Option<Instant>,
    pub interval_preview: [(ReviewRating, String); 4],

    // Add card state
    pub add_card_front: String,
    pub add_card_back: String,
    pub add_card_focus: usize, // 0 = front, 1 = back

    // Card browser state
    pub card_list_state: ListState,
    pub card_edit_mode: bool,
    pub card_edit_front: String,
    pub card_edit_back: String,
    pub card_edit_focus: usize,  // 0 = front, 1 = back
    pub card_edit_cursor: usize, // cursor position in current field
    pub card_delete_pending: bool,

    // Status message (shown temporarily)
    pub status_message: Option<(String, Instant)>,

    // Stats screen cache (computed once on entry, not every frame)
    stats_cache: Option<AggregateStats>,
}

impl App {
    pub fn new(storage: DeckStorage, config: Config) -> Self {
        let deck_list = storage.list_decks().unwrap_or_default();
        let theme = Theme::from_name(&config.theme);

        Self {
            screen: Screen::DeckSelect,
            running: true,
            config,
            theme,
            storage,
            scheduler: Scheduler::new(),
            deck_list,
            deck_list_state: ListState::default().with_selected(Some(0)),
            deck_delete_pending: false,
            deck_input: None,
            current_deck: None,
            study_queue: Vec::new(),
            current_card_idx: None,
            showing_answer: false,
            answer_revealed: false,
            cards_studied: 0,
            session_start: None,
            interval_preview: [
                (ReviewRating::Again, String::new()),
                (ReviewRating::Hard, String::new()),
                (ReviewRating::Good, String::new()),
                (ReviewRating::Easy, String::new()),
            ],
            add_card_front: String::new(),
            add_card_back: String::new(),
            add_card_focus: 0,
            // Card browser
            card_list_state: ListState::default(),
            card_edit_mode: false,
            card_edit_front: String::new(),
            card_edit_back: String::new(),
            card_edit_focus: 0,
            card_edit_cursor: 0,
            card_delete_pending: false,
            // Status
            status_message: None,
            // Stats cache
            stats_cache: None,
        }
    }

    pub fn delete_selected_deck(&mut self) {
        if let Some(i) = self.deck_list_state.selected() {
            if let Some(deck_info) = self.deck_list.get(i) {
                let deck_id = deck_info.id.clone();
                let _ = self.storage.delete_deck(&deck_id);
                self.refresh_deck_list();
                // Adjust selection if needed
                if i >= self.deck_list.len() && !self.deck_list.is_empty() {
                    self.deck_list_state.select(Some(self.deck_list.len() - 1));
                } else if self.deck_list.is_empty() {
                    self.deck_list_state.select(None);
                }
            }
        }
    }

    pub fn cycle_theme(&mut self) {
        let new_theme_name = self.theme.name.next();
        self.theme = Theme::new(new_theme_name);
        self.config.theme = new_theme_name.as_str().to_string();
        let _ = self.config.save();
    }

    /// Adjust how many new cards each session introduces (persisted to config).
    fn adjust_new_per_session(&mut self, delta: i32) {
        let current = self.config.new_per_session as i32;
        let next = (current + delta).clamp(0, 9999) as u32;
        self.config.new_per_session = next;
        let _ = self.config.save();
        self.set_status(format!("New cards per session: {}", next));
    }

    pub fn refresh_deck_list(&mut self) {
        self.deck_list = self.storage.list_decks().unwrap_or_default();
    }

    pub fn select_deck(&mut self, deck_id: &str) {
        if let Ok(Some(deck)) = self.storage.load_deck(deck_id) {
            let stats = deck.get_stats();
            self.current_deck = Some(deck);

            if stats.total_cards == 0 || (stats.new_cards == 0 && stats.due_cards == 0) {
                self.screen = Screen::AddCard;
            } else {
                self.start_study();
            }
        }
    }

    pub fn start_study(&mut self) {
        // Build study queue
        let mut queue: Vec<usize> = Vec::new();

        if let Some(ref deck) = self.current_deck {
            // Add due cards first
            for (i, card) in deck.cards.iter().enumerate() {
                if card.is_due() && !card.is_new() {
                    queue.push(i);
                }
            }

            // Add new cards (limit configurable, default 20; see config.toml
            // `new_per_session` or press +/- on the deck list)
            let new_limit = self.config.new_per_session as usize;
            let mut new_count = 0;
            for (i, card) in deck.cards.iter().enumerate() {
                if card.is_new() && new_count < new_limit {
                    queue.push(i);
                    new_count += 1;
                }
            }
        }

        if queue.is_empty() {
            // Nothing to study: don't show a bogus "session complete" screen
            self.screen = Screen::DeckSelect;
            self.current_deck = None;
            self.set_status("Nothing due right now — all caught up!".to_string());
            return;
        }

        self.study_queue = queue;
        self.cards_studied = 0;
        self.session_start = Some(Instant::now());
        self.screen = Screen::Study;

        self.next_card();
    }

    pub fn next_card(&mut self) {
        if self.study_queue.is_empty() {
            self.screen = Screen::Complete;
            return;
        }

        self.current_card_idx = Some(self.study_queue.remove(0));
        self.showing_answer = false;
        self.answer_revealed = false;

        // Update interval preview
        if let (Some(deck), Some(idx)) = (&self.current_deck, self.current_card_idx) {
            self.interval_preview = self.scheduler.preview_intervals(&deck.cards[idx]);
        }
    }

    pub fn show_answer(&mut self) {
        self.showing_answer = true;
        self.answer_revealed = true;
    }

    pub fn rate_card(&mut self, rating: ReviewRating) {
        if !self.answer_revealed {
            return;
        }

        if let (Some(ref mut deck), Some(idx)) = (&mut self.current_deck, self.current_card_idx) {
            self.scheduler.review_card(&mut deck.cards[idx], rating);
            self.cards_studied += 1;

            // If failed, add back to queue
            if rating == ReviewRating::Again {
                self.study_queue.push(idx);
            }

            // Save deck
            let _ = self.storage.save_deck(deck);

            self.next_card();
        }
    }

    pub fn add_card(&mut self) {
        if self.add_card_front.is_empty() || self.add_card_back.is_empty() {
            return;
        }

        if let Some(ref mut deck) = self.current_deck {
            deck.add_card(self.add_card_front.clone(), self.add_card_back.clone());
            let _ = self.storage.save_deck(deck);

            self.add_card_front.clear();
            self.add_card_back.clear();
            self.add_card_focus = 0;
        }
    }

    pub fn create_new_deck(&mut self, name: &str) {
        let deck = Deck::new(name.to_string());
        let _ = self.storage.save_deck(&deck);
        self.refresh_deck_list();
    }

    pub fn set_status(&mut self, message: String) {
        self.status_message = Some((message, Instant::now()));
    }

    pub fn export_backup(&mut self) {
        let path = DeckStorage::default_backup_path();
        match self.storage.export_backup(&path) {
            Ok(count) => {
                self.set_status(format!("Exported {} decks to {}", count, path.display()));
            }
            Err(e) => {
                self.set_status(format!("Export failed: {}", e));
            }
        }
    }

    pub fn import_backup(&mut self, path: &std::path::Path) {
        match self.storage.import_backup(path) {
            Ok((imported, skipped)) => {
                self.refresh_deck_list();
                if skipped > 0 {
                    self.set_status(format!(
                        "Imported {} decks ({} skipped - already exist)",
                        imported, skipped
                    ));
                } else {
                    self.set_status(format!("Imported {} decks", imported));
                }
            }
            Err(e) => {
                self.set_status(format!("Import failed: {}", e));
            }
        }
    }

    pub fn enter_card_browser(&mut self) {
        if let Some(ref deck) = self.current_deck {
            if !deck.cards.is_empty() {
                self.card_list_state = ListState::default().with_selected(Some(0));
            } else {
                self.card_list_state = ListState::default();
            }
            self.card_edit_mode = false;
            self.card_delete_pending = false;
            self.screen = Screen::CardBrowser;
        }
    }

    pub fn browse_selected_deck(&mut self) {
        if let Some(i) = self.deck_list_state.selected() {
            if let Some(deck_info) = self.deck_list.get(i) {
                if let Ok(Some(deck)) = self.storage.load_deck(&deck_info.id) {
                    self.current_deck = Some(deck);
                    self.enter_card_browser();
                }
            }
        }
    }

    pub fn start_edit_card(&mut self) {
        if let Some(i) = self.card_list_state.selected() {
            if let Some(ref deck) = self.current_deck {
                if let Some(card) = deck.cards.get(i) {
                    // Trim quotes when loading into edit fields
                    self.card_edit_front = card.front.trim_matches('"').trim().to_string();
                    self.card_edit_back = card.back.trim_matches('"').trim().to_string();
                    self.card_edit_focus = 0;
                    self.card_edit_cursor = self.card_edit_front.chars().count();
                    self.card_edit_mode = true;
                    self.card_delete_pending = false;
                }
            }
        }
    }

    pub fn save_card_edit(&mut self) {
        if let Some(i) = self.card_list_state.selected() {
            if let Some(ref mut deck) = self.current_deck {
                if let Some(card) = deck.cards.get(i) {
                    let card_id = card.id.clone();
                    deck.update_card(
                        &card_id,
                        self.card_edit_front.clone(),
                        self.card_edit_back.clone(),
                    );
                    let _ = self.storage.save_deck(deck);
                }
            }
        }
        self.card_edit_mode = false;
        self.card_edit_front.clear();
        self.card_edit_back.clear();
    }

    pub fn cancel_card_edit(&mut self) {
        self.card_edit_mode = false;
        self.card_edit_front.clear();
        self.card_edit_back.clear();
    }

    pub fn delete_selected_card(&mut self) {
        if let Some(i) = self.card_list_state.selected() {
            if let Some(ref mut deck) = self.current_deck {
                if let Some(card) = deck.cards.get(i) {
                    let card_id = card.id.clone();
                    deck.delete_card(&card_id);
                    let _ = self.storage.save_deck(deck);

                    // Adjust selection
                    if deck.cards.is_empty() {
                        self.card_list_state.select(None);
                    } else if i >= deck.cards.len() {
                        self.card_list_state.select(Some(deck.cards.len() - 1));
                    }
                }
            }
        }
        self.card_delete_pending = false;
    }

    // ══════════════════════════════════════════════════════════════════════
    // Event Handling
    // ══════════════════════════════════════════════════════════════════════

    pub fn handle_events(&mut self) -> anyhow::Result<()> {
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    return Ok(());
                }

                // Ctrl+C always quits. Other Ctrl/Alt chords are ignored so
                // they can't insert control characters into text inputs.
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    if matches!(key.code, KeyCode::Char('c')) {
                        self.running = false;
                    }
                    return Ok(());
                }
                if key.modifiers.contains(KeyModifiers::ALT) {
                    return Ok(());
                }

                match self.screen {
                    Screen::DeckSelect => self.handle_deck_select_keys(key.code),
                    Screen::Study => self.handle_study_keys(key.code),
                    Screen::AddCard => self.handle_add_card_keys(key.code),
                    Screen::CardBrowser => self.handle_card_browser_keys(key.code),
                    Screen::Stats => self.handle_stats_keys(key.code),
                    Screen::Complete => self.handle_complete_keys(key.code),
                }
            }
        }
        Ok(())
    }

    fn handle_deck_select_keys(&mut self, key: KeyCode) {
        // The inline naming dialog swallows all keys while open
        if self.deck_input.is_some() {
            self.handle_deck_input_keys(key);
            return;
        }

        if matches!(key, KeyCode::Char('d') | KeyCode::Char('D')) {
            if self.deck_delete_pending {
                self.delete_selected_deck();
                self.deck_delete_pending = false;
            } else {
                self.deck_delete_pending = true;
                self.set_status("Press d again to confirm deck deletion".to_string());
            }
            return;
        }
        self.deck_delete_pending = false;

        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Char('t') => self.cycle_theme(),
            KeyCode::Char('+') | KeyCode::Char('=') => self.adjust_new_per_session(5),
            KeyCode::Char('-') | KeyCode::Char('_') => self.adjust_new_per_session(-5),
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.deck_list_state.selected().unwrap_or(0);
                let new_i = if i == 0 {
                    self.deck_list.len().saturating_sub(1)
                } else {
                    i - 1
                };
                self.deck_list_state.select(Some(new_i));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.deck_list_state.selected().unwrap_or(0);
                let new_i = if i >= self.deck_list.len().saturating_sub(1) {
                    0
                } else {
                    i + 1
                };
                self.deck_list_state.select(Some(new_i));
            }
            KeyCode::Enter => {
                if let Some(i) = self.deck_list_state.selected() {
                    if let Some(deck_info) = self.deck_list.get(i) {
                        let deck_id = deck_info.id.clone();
                        self.select_deck(&deck_id);
                    }
                }
            }
            KeyCode::Char('n') => {
                self.deck_input = Some(DeckInput::Create {
                    name: String::new(),
                });
            }
            KeyCode::Char('i') => {
                self.deck_input = Some(DeckInput::ImportBackup {
                    path: String::new(),
                });
            }
            KeyCode::Char('r') => {
                if let Some(i) = self.deck_list_state.selected() {
                    if let Some(deck_info) = self.deck_list.get(i) {
                        self.deck_input = Some(DeckInput::Rename {
                            deck_id: deck_info.id.clone(),
                            name: deck_info.name.clone(),
                        });
                    }
                }
            }
            KeyCode::Char('b') => {
                self.browse_selected_deck();
            }
            KeyCode::Char('x') => {
                self.export_backup();
            }
            KeyCode::Char('s') => {
                self.enter_stats();
            }
            _ => {}
        }
    }

    fn handle_deck_input_keys(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => self.deck_input = None,
            KeyCode::Enter => self.confirm_deck_input(),
            KeyCode::Backspace => {
                if let Some(input) = self.deck_input.as_mut() {
                    input.value_mut().pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(input) = self.deck_input.as_mut() {
                    input.value_mut().push(c);
                }
            }
            _ => {}
        }
    }

    fn confirm_deck_input(&mut self) {
        if let Some(input) = self.deck_input.take() {
            let value = input.value().trim().to_string();
            if value.is_empty() {
                self.set_status("Input cannot be empty".to_string());
                return;
            }

            match input {
                DeckInput::Create { .. } => {
                    if self.storage.deck_name_exists(&value) {
                        self.set_status(format!("Deck '{}' already exists", value));
                    } else {
                        self.create_new_deck(&value);
                        self.set_status(format!("Created deck '{}'", value));
                    }
                }
                DeckInput::Rename { deck_id, .. } => {
                    if let Ok(Some(mut deck)) = self.storage.load_deck(&deck_id) {
                        deck.name = value.clone();
                        let _ = self.storage.save_deck(&deck);
                        self.refresh_deck_list();
                        self.set_status(format!("Renamed deck to '{}'", value));
                    }
                }
                DeckInput::ImportBackup { .. } => {
                    self.import_backup(std::path::Path::new(&value));
                }
            }
        }
    }

    fn handle_study_keys(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.screen = Screen::DeckSelect;
                self.current_deck = None;
            }
            KeyCode::Char('t') => self.cycle_theme(),
            KeyCode::Char(' ') => {
                if !self.showing_answer {
                    self.show_answer();
                } else {
                    // Toggle back to front (answer_revealed stays true)
                    self.showing_answer = false;
                }
            }
            KeyCode::Char('a') => {
                self.screen = Screen::AddCard;
            }
            KeyCode::Char('b') => {
                self.enter_card_browser();
            }
            KeyCode::Char(c) => {
                if let Some(rating) = ReviewRating::from_key(c) {
                    self.rate_card(rating);
                }
            }
            _ => {}
        }
    }

    fn handle_add_card_keys(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                if let Some(ref deck) = self.current_deck {
                    if deck.cards.is_empty() {
                        self.screen = Screen::DeckSelect;
                        self.current_deck = None;
                    } else {
                        self.start_study();
                    }
                } else {
                    self.screen = Screen::DeckSelect;
                }
            }
            KeyCode::Tab => {
                self.add_card_focus = (self.add_card_focus + 1) % 2;
            }
            KeyCode::Enter => {
                if self.add_card_focus == 0 {
                    self.add_card_focus = 1;
                } else {
                    self.add_card();
                }
            }
            KeyCode::Char(c) => {
                if self.add_card_focus == 0 {
                    self.add_card_front.push(c);
                } else {
                    self.add_card_back.push(c);
                }
            }
            KeyCode::Backspace => {
                if self.add_card_focus == 0 {
                    self.add_card_front.pop();
                } else {
                    self.add_card_back.pop();
                }
            }
            _ => {}
        }
    }

    fn handle_complete_keys(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                self.screen = Screen::DeckSelect;
                self.current_deck = None;
                self.refresh_deck_list();
            }
            _ => {}
        }
    }

    fn handle_stats_keys(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.stats_cache = None;
                self.screen = Screen::DeckSelect;
            }
            KeyCode::Char('t') => self.cycle_theme(),
            _ => {}
        }
    }

    /// Enter the stats screen, computing aggregate statistics once.
    pub fn enter_stats(&mut self) {
        self.refresh_deck_list();
        self.stats_cache = Some(self.compute_aggregate_stats());
        self.screen = Screen::Stats;
    }

    fn compute_aggregate_stats(&self) -> AggregateStats {
        let mut agg = AggregateStats::default();
        let mut review_dates: Vec<chrono::NaiveDate> = Vec::new();

        for deck_info in &self.deck_list {
            if let Ok(Some(deck)) = self.storage.load_deck(&deck_info.id) {
                for card in &deck.cards {
                    agg.total_cards += 1;
                    agg.total_reviews += card.total_reviews;

                    // Collect review dates for streak calculation
                    if let Some(reviewed) = card.last_reviewed {
                        review_dates.push(reviewed.date_naive());
                    }

                    // Categorize by ease factor
                    if card.is_new() {
                        agg.ease.new += 1;
                    } else if card.ease_factor >= 2.5 {
                        agg.ease.easy += 1;
                    } else if card.ease_factor >= 2.0 {
                        agg.ease.good += 1;
                    } else if card.ease_factor >= 1.5 {
                        agg.ease.hard += 1;
                    } else {
                        agg.ease.struggling += 1;
                    }
                }
            }
        }

        let (daily_streak, weekly_streak) = calculate_streaks(&review_dates);
        agg.daily_streak = daily_streak;
        agg.weekly_streak = weekly_streak;
        agg
    }

    fn handle_card_browser_keys(&mut self, key: KeyCode) {
        if self.card_edit_mode {
            // Edit mode - calculate field length first (before mutable borrow)
            let field_len = if self.card_edit_focus == 0 {
                self.card_edit_front.chars().count()
            } else {
                self.card_edit_back.chars().count()
            };

            match key {
                KeyCode::Esc => {
                    self.cancel_card_edit();
                }
                KeyCode::Tab => {
                    // Switch field and set cursor to end of new field
                    self.card_edit_focus = (self.card_edit_focus + 1) % 2;
                    self.card_edit_cursor = if self.card_edit_focus == 0 {
                        self.card_edit_front.chars().count()
                    } else {
                        self.card_edit_back.chars().count()
                    };
                }
                KeyCode::Enter => {
                    self.save_card_edit();
                }
                KeyCode::Left => {
                    if self.card_edit_cursor > 0 {
                        self.card_edit_cursor -= 1;
                    }
                }
                KeyCode::Right => {
                    if self.card_edit_cursor < field_len {
                        self.card_edit_cursor += 1;
                    }
                }
                KeyCode::Home => {
                    self.card_edit_cursor = 0;
                }
                KeyCode::End => {
                    self.card_edit_cursor = field_len;
                }
                KeyCode::Char(c) => {
                    // Insert character at cursor position
                    let field = if self.card_edit_focus == 0 {
                        &mut self.card_edit_front
                    } else {
                        &mut self.card_edit_back
                    };
                    let byte_pos = field
                        .char_indices()
                        .nth(self.card_edit_cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(field.len());
                    field.insert(byte_pos, c);
                    self.card_edit_cursor += 1;
                }
                KeyCode::Backspace => {
                    if self.card_edit_cursor > 0 {
                        // Remove character before cursor
                        let field = if self.card_edit_focus == 0 {
                            &mut self.card_edit_front
                        } else {
                            &mut self.card_edit_back
                        };
                        let byte_pos = field
                            .char_indices()
                            .nth(self.card_edit_cursor - 1)
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        field.remove(byte_pos);
                        self.card_edit_cursor -= 1;
                    }
                }
                KeyCode::Delete if self.card_edit_cursor < field_len => {
                    // Remove character at cursor
                    let field = if self.card_edit_focus == 0 {
                        &mut self.card_edit_front
                    } else {
                        &mut self.card_edit_back
                    };
                    let byte_pos = field
                        .char_indices()
                        .nth(self.card_edit_cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(field.len());
                    if byte_pos < field.len() {
                        field.remove(byte_pos);
                    }
                }
                _ => {}
            }
        } else {
            // Browse mode
            match key {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.screen = Screen::DeckSelect;
                    self.current_deck = None;
                    self.refresh_deck_list();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.card_delete_pending = false;
                    if let Some(ref deck) = self.current_deck {
                        if !deck.cards.is_empty() {
                            let i = self.card_list_state.selected().unwrap_or(0);
                            let new_i = if i == 0 { deck.cards.len() - 1 } else { i - 1 };
                            self.card_list_state.select(Some(new_i));
                        }
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.card_delete_pending = false;
                    if let Some(ref deck) = self.current_deck {
                        if !deck.cards.is_empty() {
                            let i = self.card_list_state.selected().unwrap_or(0);
                            let new_i = if i >= deck.cards.len() - 1 { 0 } else { i + 1 };
                            self.card_list_state.select(Some(new_i));
                        }
                    }
                }
                KeyCode::Char('e') => {
                    self.card_delete_pending = false;
                    self.start_edit_card();
                }
                KeyCode::Char('d') => {
                    if self.card_delete_pending {
                        self.delete_selected_card();
                    } else {
                        self.card_delete_pending = true;
                    }
                }
                KeyCode::Char('a') => {
                    self.card_delete_pending = false;
                    self.screen = Screen::AddCard;
                }
                KeyCode::Char('t') => {
                    self.card_delete_pending = false;
                    self.cycle_theme();
                }
                _ => {
                    self.card_delete_pending = false;
                }
            }
        }
    }

    // ══════════════════════════════════════════════════════════════════════
    // Rendering
    // ══════════════════════════════════════════════════════════════════════

    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();

        // Clear with background
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default().style(Style::default().bg(self.theme.colors.bg_dark)),
            area,
        );

        match self.screen {
            Screen::DeckSelect => self.render_deck_select(frame, area),
            Screen::Study => self.render_study(frame, area),
            Screen::AddCard => self.render_add_card(frame, area),
            Screen::CardBrowser => self.render_card_browser(frame, area),
            Screen::Stats => self.render_stats(frame, area),
            Screen::Complete => self.render_complete(frame, area),
        }
    }

    fn render_deck_select(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Length(2), // Header bar
            Constraint::Min(5),    // Deck list
            Constraint::Length(3), // Help
        ])
        .split(area);

        // Header
        let header_ctx = format!(
            "{} decks · new/day {} · [{}]",
            self.deck_list.len(),
            self.config.new_per_session,
            self.theme.name.display_name()
        );
        frame.render_widget(HeaderBar::new(&self.theme, "SRL", &header_ctx), chunks[0]);

        // Deck list
        let list_area = centered_rect(70, 100, chunks[1]);

        let items: Vec<ListItem> = self
            .deck_list
            .iter()
            .map(|deck| {
                let content = Line::from(vec![
                    Span::styled(
                        deck.name.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  ·  {} cards", deck.card_count),
                        Style::default().fg(self.theme.colors.text_muted),
                    ),
                ]);
                ListItem::new(content)
            })
            .collect();

        let list = List::new(items)
            .highlight_style(self.theme.selected())
            .highlight_symbol("▌ ");

        frame.render_stateful_widget(list, list_area, &mut self.deck_list_state);

        // Key hints with theme indicator
        let theme_hint = format!("[{}]", self.theme.name.display_name());
        let hints_data: [(&str, &str); 12] = [
            ("j/k", "nav"),
            ("Enter", "study"),
            ("b", "browse"),
            ("n", "new"),
            ("r", "rename"),
            ("i", "import"),
            ("d", "del 2x"),
            ("x", "export"),
            ("s", "stats"),
            ("+/-", "new/day"),
            ("t", &theme_hint),
            ("q", "quit"),
        ];
        let hints = KeyHints::new(&hints_data, &self.theme);
        frame.render_widget(hints, chunks[2]);

        // Show status message if recent (within 5 seconds)
        if let Some((ref msg, time)) = self.status_message {
            if time.elapsed().as_secs() < 5 {
                let status = Paragraph::new(msg.as_str())
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(self.theme.colors.success));
                // Render above the hints
                let status_area = Rect {
                    x: chunks[2].x,
                    y: chunks[2].y.saturating_sub(1),
                    width: chunks[2].width,
                    height: 1,
                };
                frame.render_widget(status, status_area);
            }
        }

        // Inline deck naming dialog (create / rename), rendered last so it
        // sits on top of the list
        if self.deck_input.is_some() {
            self.render_deck_input_dialog(frame, area);
        }
    }

    fn render_deck_input_dialog(&mut self, frame: &mut Frame, area: Rect) {
        let (title, value) = {
            let input = self.deck_input.as_ref().expect("checked by caller");
            (input.title(), input.value().to_string())
        };
        let placeholder = match self.deck_input.as_ref().expect("checked by caller") {
            DeckInput::ImportBackup { .. } => "Path to backup JSON…".to_string(),
            _ => "Type a deck name…".to_string(),
        };

        let width = 50.min(area.width);
        let height = 5.min(area.height);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };

        frame.render_widget(Clear, popup);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(self.theme.colors.accent))
            .title(title)
            .title_style(self.theme.highlight());
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let hint_line = Line::from(Span::styled(
            "Enter to confirm · Esc to cancel",
            Style::default().fg(self.theme.colors.text_dim),
        ));

        let (name_text, name_style) = if value.is_empty() {
            (placeholder, Style::default().fg(self.theme.colors.text_dim))
        } else {
            (value.clone(), Style::default().fg(self.theme.colors.text))
        };

        if inner.height >= 2 && inner.width > 0 {
            frame.render_widget(
                Paragraph::new(name_text).style(name_style),
                Rect { height: 1, ..inner },
            );
            frame.render_widget(
                hint_line,
                Rect {
                    y: inner.y + inner.height - 1,
                    height: 1,
                    ..inner
                },
            );
        }

        // Place the terminal cursor at the end of the typed value
        if !value.is_empty() || inner.width > 1 {
            let cursor_x = inner.x + unicode_width::UnicodeWidthStr::width(value.as_str()) as u16;
            let cursor_x = cursor_x.min(inner.x + inner.width.saturating_sub(1));
            frame.set_cursor_position((cursor_x, inner.y));
        }
    }

    fn render_study(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Length(2), // Header bar (deck name + session progress)
            Constraint::Length(1), // Stats
            Constraint::Length(1), // Spacing
            Constraint::Min(8),    // Card
            Constraint::Length(2), // Rating buttons (borderless)
            Constraint::Length(1), // Spacing
            Constraint::Length(2), // Hints
        ])
        .split(area);

        // Header with deck name and session progress
        if let Some(ref deck) = self.current_deck {
            let done = self.cards_studied;
            let in_hand = usize::from(self.current_card_idx.is_some());
            let left = self.study_queue.len() + in_hand;
            let total = done + left;

            let progress = if total > 0 {
                let bar_width = 10usize;
                let filled = (done * bar_width)
                    .checked_div(total)
                    .unwrap_or(0)
                    .min(bar_width);
                let bar: String =
                    format!("{}{}", "▰".repeat(filled), "▱".repeat(bar_width - filled));
                format!("{} {}/{}", bar, done, total)
            } else {
                String::new()
            };

            frame.render_widget(
                HeaderBar::new(&self.theme, &deck.name, &progress),
                chunks[0],
            );

            // Stats bar
            let stats = deck.get_stats();
            frame.render_widget(StatsBar::new(stats, &self.theme), chunks[1]);
        }

        // Card display
        let card_area = centered_rect(85, 100, chunks[3]);

        if let (Some(ref deck), Some(idx)) = (&self.current_deck, self.current_card_idx) {
            let card = &deck.cards[idx];
            let (content, is_front) = if self.showing_answer {
                (&card.back, false)
            } else {
                (&card.front, true)
            };

            frame.render_widget(
                FlashcardWidget::new(content, is_front, &self.theme),
                card_area,
            );
        }

        // Rating buttons
        let buttons_area = centered_rect(90, 100, chunks[4]);
        frame.render_widget(
            RatingButtons::new(&self.interval_preview, self.answer_revealed, &self.theme),
            buttons_area,
        );

        // Key hints
        let hints = if self.answer_revealed {
            KeyHints::new(
                &[
                    ("Space", "flip"),
                    ("1", "Again"),
                    ("2", "Hard"),
                    ("3", "Good"),
                    ("4", "Easy"),
                    ("Esc", "quit"),
                ],
                &self.theme,
            )
        } else {
            KeyHints::new(
                &[
                    ("Space", "show answer"),
                    ("a", "add"),
                    ("b", "browse"),
                    ("Esc", "quit"),
                ],
                &self.theme,
            )
        };
        frame.render_widget(hints, chunks[6]);
    }

    fn render_add_card(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Length(2), // Header bar
            Constraint::Length(1), // Spacing
            Constraint::Length(3), // Front label + input
            Constraint::Length(1), // Spacing
            Constraint::Length(3), // Back label + input
            Constraint::Length(2), // Spacing
            Constraint::Length(3), // Button
            Constraint::Min(1),    // Spacer
            Constraint::Length(2), // Hints
        ])
        .split(centered_rect(60, 100, area));

        // Title
        let deck_name = self
            .current_deck
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("Deck");
        frame.render_widget(
            HeaderBar::new(&self.theme, "Add Card", deck_name),
            chunks[0],
        );

        // Front input
        let front_style = if self.add_card_focus == 0 {
            Style::default().fg(self.theme.colors.accent)
        } else {
            Style::default().fg(self.theme.colors.text_muted)
        };
        let front = Paragraph::new(self.add_card_front.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(front_style)
                .title(" Front (Question) ")
                .title_style(front_style),
        );
        frame.render_widget(front, chunks[2]);

        // Back input
        let back_style = if self.add_card_focus == 1 {
            Style::default().fg(self.theme.colors.accent)
        } else {
            Style::default().fg(self.theme.colors.text_muted)
        };
        let back = Paragraph::new(self.add_card_back.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(back_style)
                .title(" Back (Answer) ")
                .title_style(back_style),
        );
        frame.render_widget(back, chunks[4]);

        // Card count
        let count = self
            .current_deck
            .as_ref()
            .map(|d| d.cards.len())
            .unwrap_or(0);
        let status = Paragraph::new(format!("Cards: {}", count))
            .alignment(Alignment::Center)
            .style(Style::default().fg(self.theme.colors.text_muted));
        frame.render_widget(status, chunks[6]);

        // Hints
        let hints = KeyHints::new(
            &[
                ("Tab", "switch field"),
                ("Enter", "add card"),
                ("Esc", "done"),
            ],
            &self.theme,
        );
        frame.render_widget(hints, chunks[8]);
    }

    fn render_card_browser(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Length(2), // Header bar
            Constraint::Length(1), // Spacing
            Constraint::Min(10),   // Main content
            Constraint::Length(2), // Hints
        ])
        .split(area);

        // Header with deck name and card count
        let deck_name = self
            .current_deck
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("Cards");
        let card_count = self
            .current_deck
            .as_ref()
            .map(|d| d.cards.len())
            .unwrap_or(0);
        let ctx = format!("{} cards", card_count);
        frame.render_widget(HeaderBar::new(&self.theme, deck_name, &ctx), chunks[0]);

        // Main content: split into list and detail
        let main_chunks = Layout::horizontal([
            Constraint::Percentage(35), // Card list
            Constraint::Percentage(65), // Card details
        ])
        .split(chunks[2]);

        // Card list
        if let Some(ref deck) = self.current_deck {
            let items: Vec<ListItem> = deck
                .cards
                .iter()
                .map(|card| {
                    let front_preview =
                        truncate_display_width(card.front.trim_matches('"').trim(), 22);
                    let status = if card.is_new() {
                        "(new)".to_string()
                    } else if card.is_due() {
                        "(due)".to_string()
                    } else if card.interval == 1 {
                        "(1d)".to_string()
                    } else {
                        format!("({}d)", card.interval)
                    };
                    let content = Line::from(vec![
                        Span::styled(front_preview, Style::default().fg(self.theme.colors.text)),
                        Span::styled(
                            format!(" {}", status),
                            Style::default().fg(self.theme.colors.text_muted),
                        ),
                    ]);
                    ListItem::new(content)
                })
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(self.theme.colors.primary))
                        .title(" Cards ")
                        .title_style(self.theme.highlight()),
                )
                .highlight_style(self.theme.selected())
                .highlight_symbol("> ");

            frame.render_stateful_widget(list, main_chunks[0], &mut self.card_list_state);

            // Card details panel
            if let Some(idx) = self.card_list_state.selected() {
                if let Some(card) = deck.cards.get(idx) {
                    self.render_card_details(frame, main_chunks[1], card);
                }
            }
        }

        // Key hints
        let hints = if self.card_edit_mode {
            KeyHints::new(
                &[("Tab", "switch"), ("Enter", "save"), ("Esc", "cancel")],
                &self.theme,
            )
        } else if self.card_delete_pending {
            KeyHints::new(&[("d", "confirm delete"), ("any", "cancel")], &self.theme)
        } else {
            KeyHints::new(
                &[
                    ("j/k", "nav"),
                    ("e", "edit"),
                    ("d", "delete"),
                    ("a", "add"),
                    ("Esc", "back"),
                ],
                &self.theme,
            )
        };
        frame.render_widget(hints, chunks[3]);
    }

    fn render_card_details(&self, frame: &mut Frame, area: Rect, card: &crate::models::Card) {
        let chunks = Layout::vertical([
            Constraint::Length(5), // Front
            Constraint::Length(1), // Spacing
            Constraint::Min(8),    // Back - larger to show more content
            Constraint::Length(1), // Spacing
            Constraint::Length(7), // Metadata
        ])
        .split(area);

        if self.card_edit_mode {
            // Edit mode - show editable fields with real blinking cursor
            let front_style = if self.card_edit_focus == 0 {
                Style::default().fg(self.theme.colors.accent)
            } else {
                Style::default().fg(self.theme.colors.text_muted)
            };
            let front = Paragraph::new(self.card_edit_front.as_str())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(front_style)
                        .title(" Front (editing) ")
                        .title_style(front_style),
                )
                .wrap(ratatui::widgets::Wrap { trim: true });
            frame.render_widget(front, chunks[0]);

            // Set cursor position for front field (accounting for wrap)
            if self.card_edit_focus == 0 {
                let inner_width = chunks[0].width.saturating_sub(2) as usize; // -2 for borders
                let cursor_pos = self.card_edit_cursor;
                let row = cursor_pos.checked_div(inner_width).unwrap_or(0);
                let col = cursor_pos.checked_rem(inner_width).unwrap_or(0);
                let cursor_x = chunks[0].x + 1 + col as u16;
                let cursor_y = chunks[0].y + 1 + row as u16;
                // On tiny terminals the wrapped row can fall outside the
                // field; only place the cursor when it stays inside.
                if cursor_y < chunks[0].bottom() && cursor_x < chunks[0].right() {
                    frame.set_cursor_position((cursor_x, cursor_y));
                }
            }

            let back_style = if self.card_edit_focus == 1 {
                Style::default().fg(self.theme.colors.accent)
            } else {
                Style::default().fg(self.theme.colors.text_muted)
            };
            let back = Paragraph::new(self.card_edit_back.as_str())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(back_style)
                        .title(" Back (editing) ")
                        .title_style(back_style),
                )
                .wrap(ratatui::widgets::Wrap { trim: true });
            frame.render_widget(back, chunks[2]);

            // Set cursor position for back field (accounting for wrap)
            if self.card_edit_focus == 1 {
                let inner_width = chunks[2].width.saturating_sub(2) as usize; // -2 for borders
                let cursor_pos = self.card_edit_cursor;
                let row = cursor_pos.checked_div(inner_width).unwrap_or(0);
                let col = cursor_pos.checked_rem(inner_width).unwrap_or(0);
                let cursor_x = chunks[2].x + 1 + col as u16;
                let cursor_y = chunks[2].y + 1 + row as u16;
                if cursor_y < chunks[2].bottom() && cursor_x < chunks[2].right() {
                    frame.set_cursor_position((cursor_x, cursor_y));
                }
            }
        } else {
            // View mode - trim quotes from display
            let front_text = card.front.trim_matches('"').trim();
            let front = Paragraph::new(front_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(self.theme.colors.accent))
                        .title(" Front ")
                        .title_style(Style::default().fg(self.theme.colors.accent)),
                )
                .wrap(ratatui::widgets::Wrap { trim: true });
            frame.render_widget(front, chunks[0]);

            let back_text = card.back.trim_matches('"').trim();
            let back = Paragraph::new(back_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(self.theme.colors.success))
                        .title(" Back ")
                        .title_style(Style::default().fg(self.theme.colors.success)),
                )
                .wrap(ratatui::widgets::Wrap { trim: true });
            frame.render_widget(back, chunks[2]);
        }

        // Metadata
        let due_str = match card.due_date {
            None => "New card".to_string(),
            Some(due) => {
                let now = chrono::Local::now();
                let diff = due.signed_duration_since(now);
                let days = diff.num_days();
                if days < 0 {
                    format!("Overdue by {} days", -days)
                } else if days == 0 {
                    "Due today".to_string()
                } else if days == 1 {
                    "Due tomorrow".to_string()
                } else {
                    format!("Due in {} days", days)
                }
            }
        };

        let metadata = vec![
            Line::from(vec![
                Span::styled(
                    "Status: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(&due_str, Style::default().fg(self.theme.colors.primary)),
            ]),
            Line::from(vec![
                Span::styled(
                    "Interval: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(
                    format!("{} days", card.interval),
                    Style::default().fg(self.theme.colors.text),
                ),
            ]),
            Line::from(vec![
                Span::styled("Ease: ", Style::default().fg(self.theme.colors.text_muted)),
                Span::styled(
                    format!("{:.2}", card.ease_factor),
                    Style::default().fg(self.theme.colors.text),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Reviews: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(
                    card.total_reviews.to_string(),
                    Style::default().fg(self.theme.colors.text),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Lapses: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(
                    card.lapses.to_string(),
                    Style::default().fg(self.theme.colors.rating_again),
                ),
            ]),
        ];

        let metadata_block = Paragraph::new(metadata).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(self.theme.colors.text_dim))
                .title(" Stats ")
                .title_style(Style::default().fg(self.theme.colors.text_muted)),
        );
        frame.render_widget(metadata_block, chunks[4]);
    }

    fn render_stats(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([
            Constraint::Length(2), // Header bar
            Constraint::Length(1), // Spacing
            Constraint::Min(10),   // Stats content
            Constraint::Length(2), // Hints
        ])
        .split(area);

        // Header
        let stats_ctx = self
            .stats_cache
            .as_ref()
            .map(|a| format!("{} cards · {} reviews", a.total_cards, a.total_reviews))
            .unwrap_or_default();
        frame.render_widget(
            HeaderBar::new(&self.theme, "Statistics", &stats_ctx),
            chunks[0],
        );

        // Calculate aggregate stats from the cache computed on entry
        let agg = self.stats_cache.clone().unwrap_or_default();
        let total_reviews = agg.total_reviews;
        let total_cards = agg.total_cards;
        let ease_counts = agg.ease;
        let (daily_streak, weekly_streak) = (agg.daily_streak, agg.weekly_streak);

        // Main content area
        let content_area = centered_rect(70, 100, chunks[2]);
        let stat_chunks = Layout::vertical([
            Constraint::Length(7), // Overview stats
            Constraint::Length(1), // Spacing
            Constraint::Min(8),    // Ease breakdown
        ])
        .split(content_area);

        // Overview stats
        let overview_lines = vec![
            Line::from(vec![
                Span::styled(
                    "Total Cards: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(
                    total_cards.to_string(),
                    Style::default()
                        .fg(self.theme.colors.primary)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Total Reviews: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(
                    total_reviews.to_string(),
                    Style::default()
                        .fg(self.theme.colors.primary)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "Daily Streak: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(
                    format!(
                        "{} day{}",
                        daily_streak,
                        if daily_streak == 1 { "" } else { "s" }
                    ),
                    Style::default().fg(if daily_streak > 0 {
                        self.theme.colors.success
                    } else {
                        self.theme.colors.text_dim
                    }),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Weekly Streak: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(
                    format!(
                        "{} week{}",
                        weekly_streak,
                        if weekly_streak == 1 { "" } else { "s" }
                    ),
                    Style::default().fg(if weekly_streak > 0 {
                        self.theme.colors.success
                    } else {
                        self.theme.colors.text_dim
                    }),
                ),
            ]),
        ];

        let overview = Paragraph::new(overview_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(self.theme.colors.primary))
                .title(" Overview ")
                .title_style(self.theme.highlight()),
        );
        frame.render_widget(overview, stat_chunks[0]);

        // Ease level breakdown
        let ease_lines = vec![
            Line::from(vec![
                Span::styled("New: ", Style::default().fg(self.theme.colors.text_muted)),
                Span::styled(
                    ease_counts.new.to_string(),
                    Style::default().fg(self.theme.colors.accent),
                ),
                Span::styled(
                    " cards not yet studied",
                    Style::default().fg(self.theme.colors.text_dim),
                ),
            ]),
            Line::from(vec![
                Span::styled("Easy: ", Style::default().fg(self.theme.colors.text_muted)),
                Span::styled(
                    ease_counts.easy.to_string(),
                    Style::default().fg(self.theme.colors.rating_easy),
                ),
                Span::styled(
                    " cards (ease >= 2.5)",
                    Style::default().fg(self.theme.colors.text_dim),
                ),
            ]),
            Line::from(vec![
                Span::styled("Good: ", Style::default().fg(self.theme.colors.text_muted)),
                Span::styled(
                    ease_counts.good.to_string(),
                    Style::default().fg(self.theme.colors.rating_good),
                ),
                Span::styled(
                    " cards (ease 2.0-2.5)",
                    Style::default().fg(self.theme.colors.text_dim),
                ),
            ]),
            Line::from(vec![
                Span::styled("Hard: ", Style::default().fg(self.theme.colors.text_muted)),
                Span::styled(
                    ease_counts.hard.to_string(),
                    Style::default().fg(self.theme.colors.rating_hard),
                ),
                Span::styled(
                    " cards (ease 1.5-2.0)",
                    Style::default().fg(self.theme.colors.text_dim),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Struggling: ",
                    Style::default().fg(self.theme.colors.text_muted),
                ),
                Span::styled(
                    ease_counts.struggling.to_string(),
                    Style::default().fg(self.theme.colors.rating_again),
                ),
                Span::styled(
                    " cards (ease < 1.5)",
                    Style::default().fg(self.theme.colors.text_dim),
                ),
            ]),
        ];

        let ease_block = Paragraph::new(ease_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(self.theme.colors.accent))
                .title(" Cards by Difficulty ")
                .title_style(Style::default().fg(self.theme.colors.accent)),
        );
        frame.render_widget(ease_block, stat_chunks[2]);

        // Key hints
        let hints = KeyHints::new(&[("t", "theme"), ("Esc", "back")], &self.theme);
        frame.render_widget(hints, chunks[3]);
    }

    fn render_complete(&mut self, frame: &mut Frame, area: Rect) {
        let card_area = centered_rect(50, 40, area);

        let duration_mins = self
            .session_start
            .map(|s| s.elapsed().as_secs() / 60)
            .unwrap_or(0);

        frame.render_widget(
            CompletionScreen::new(self.cards_studied, duration_mins, &self.theme),
            card_area,
        );
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Helper Functions
// ══════════════════════════════════════════════════════════════════════════

/// Create a centered rectangle.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

/// Counts of cards at each ease level.
#[derive(Debug, Default, Clone)]
struct EaseLevelCounts {
    new: usize,
    easy: usize,
    good: usize,
    hard: usize,
    struggling: usize,
}

/// Aggregated statistics for the stats screen, computed once on entry
/// instead of reloading every deck from disk on every frame.
#[derive(Debug, Default, Clone)]
struct AggregateStats {
    total_cards: usize,
    total_reviews: u32,
    daily_streak: u32,
    weekly_streak: u32,
    ease: EaseLevelCounts,
}

/// Truncate a string to at most `max_width` display columns (unicode aware),
/// appending an ellipsis when truncation happens.
fn truncate_display_width(s: &str, max_width: usize) -> String {
    use unicode_width::UnicodeWidthStr;

    let mut width = 0usize;
    let mut out = String::new();
    for c in s.chars() {
        let cw = UnicodeWidthStr::width(c.to_string().as_str());
        if width + cw > max_width {
            out.push('…');
            return out;
        }
        width += cw;
        out.push(c);
    }
    out
}

/// Calculate daily and weekly streaks from review dates.
fn calculate_streaks(review_dates: &[chrono::NaiveDate]) -> (u32, u32) {
    calculate_streaks_on(chrono::Local::now().date_naive(), review_dates)
}

/// Same, with an injectable "today" so tests can pin the date.
fn calculate_streaks_on(
    today: chrono::NaiveDate,
    review_dates: &[chrono::NaiveDate],
) -> (u32, u32) {
    use chrono::Datelike;
    use std::collections::HashSet;

    if review_dates.is_empty() {
        return (0, 0);
    }

    let unique_dates: HashSet<_> = review_dates.iter().cloned().collect();

    // Daily streak: consecutive days ending today or yesterday
    let mut daily_streak = 0u32;
    let mut check_date = today;

    // Allow starting from yesterday if no reviews today
    if !unique_dates.contains(&today) {
        check_date = today - chrono::Duration::days(1);
        if !unique_dates.contains(&check_date) {
            // No reviews today or yesterday, streak is 0
            check_date = today; // Reset so the loop doesn't count anything
        }
    }

    while unique_dates.contains(&check_date) {
        daily_streak += 1;
        check_date -= chrono::Duration::days(1);
    }

    // Weekly streak: consecutive weeks with at least one review
    // A week is Mon-Sun, count weeks ending with current or previous week
    let mut weekly_streak = 0u32;

    // Get the Monday of current week
    let days_since_monday = today.weekday().num_days_from_monday();
    let mut week_start = today - chrono::Duration::days(days_since_monday as i64);

    // Check if current week has reviews
    let current_week_has_reviews = (0..7).any(|d| {
        let day = week_start + chrono::Duration::days(d);
        unique_dates.contains(&day)
    });

    if !current_week_has_reviews {
        // Check previous week
        week_start -= chrono::Duration::days(7);
    }

    // Count consecutive weeks
    loop {
        let week_has_reviews = (0..7).any(|d| {
            let day = week_start + chrono::Duration::days(d);
            unique_dates.contains(&day)
        });

        if week_has_reviews {
            weekly_streak += 1;
            week_start -= chrono::Duration::days(7);
        } else {
            break;
        }
    }

    (daily_streak, weekly_streak)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-08-29 is a Saturday; Monday of that week is 2026-08-24.
    fn d(y: i32, m: u32, day: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn empty_dates_no_streak() {
        let today = d(2026, 8, 29);
        assert_eq!(calculate_streaks_on(today, &[]), (0, 0));
    }

    #[test]
    fn review_today_starts_streak() {
        let today = d(2026, 8, 29);
        let dates = vec![today];
        assert_eq!(calculate_streaks_on(today, &dates), (1, 1));
    }

    #[test]
    fn consecutive_days_count_up() {
        let today = d(2026, 8, 29);
        let dates = vec![
            today,
            today - chrono::Duration::days(1),
            today - chrono::Duration::days(2),
        ];
        assert_eq!(calculate_streaks_on(today, &dates), (3, 1));
    }

    #[test]
    fn streak_continues_from_yesterday() {
        let today = d(2026, 8, 29);
        let dates = vec![
            today - chrono::Duration::days(1),
            today - chrono::Duration::days(2),
        ];
        assert_eq!(calculate_streaks_on(today, &dates), (2, 1));
    }

    #[test]
    fn two_day_gap_resets_daily_streak() {
        let today = d(2026, 8, 29);
        let dates = vec![today - chrono::Duration::days(2)];
        assert_eq!(calculate_streaks_on(today, &dates), (0, 1));
    }

    #[test]
    fn weekly_streak_spans_weeks() {
        let today = d(2026, 8, 29); // Saturday
                                    // Reviews 1, 2 and 3 weeks ago (in previous Mon-Sun weeks), none this week
        let dates = vec![
            today - chrono::Duration::days(8),
            today - chrono::Duration::days(10),
            today - chrono::Duration::days(15),
        ];
        let (_, weekly) = calculate_streaks_on(today, &dates);
        assert_eq!(weekly, 2, "previous week and the one before it");
    }

    #[test]
    fn duplicate_reviews_same_day_count_once() {
        let today = d(2026, 8, 29);
        let dates = vec![today; 10];
        assert_eq!(calculate_streaks_on(today, &dates), (1, 1));
    }

    #[test]
    fn truncate_display_width_unicode() {
        assert_eq!(truncate_display_width("hello", 10), "hello");
        assert_eq!(truncate_display_width("hello world", 8), "hello wo…");
        // CJK chars are 2 columns wide each: 3 chars = 6 columns
        assert_eq!(truncate_display_width("你好世界", 7), "你好世…");
        assert_eq!(truncate_display_width("你好", 6), "你好");
    }

    // ── render smoke tests (TestBackend) ────────────────────────────────

    use crate::models::Deck;
    use ratatui::{backend::TestBackend, Terminal};

    fn make_app() -> App {
        let dir = std::env::temp_dir().join(format!("srl_ui_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = DeckStorage::new(dir).unwrap();
        App::new(storage, Config::default())
    }

    fn draw_at(app: &mut App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol().to_string())
            .collect()
    }

    #[test]
    fn renders_all_screens_across_terminal_sizes() {
        let mut app = make_app();

        let mut deck = Deck::new("UI Deck".into());
        deck.add_card("What is SRL?".into(), "Spaced repetition learning".into());
        deck.add_card(
            "A longer question with more text to wrap around".into(),
            "multi\nline\nanswer".into(),
        );
        app.storage.save_deck(&deck).unwrap();
        app.refresh_deck_list();

        // Exercise every screen at desktop, small-board (60x20 ≈ 480x320)
        // and extremely tiny sizes; nothing may panic.
        for (w, h) in [(80u16, 24u16), (60, 20), (40, 14), (20, 8)] {
            app.screen = Screen::DeckSelect;
            draw_at(&mut app, w, h);

            app.select_deck(&deck.id); // -> Study with a fresh queue
            draw_at(&mut app, w, h);

            app.show_answer();
            draw_at(&mut app, w, h);

            app.screen = Screen::AddCard;
            draw_at(&mut app, w, h);

            app.screen = Screen::CardBrowser;
            app.enter_card_browser();
            draw_at(&mut app, w, h);

            app.enter_stats();
            draw_at(&mut app, w, h);

            app.screen = Screen::Complete;
            draw_at(&mut app, w, h);

            app.screen = Screen::DeckSelect;
            app.deck_input = Some(DeckInput::Create {
                name: String::new(),
            });
            draw_at(&mut app, w, h);
            app.deck_input = None;
        }
    }

    #[test]
    fn deck_select_shows_header_and_decks() {
        let mut app = make_app();
        let mut deck = Deck::new("UI Deck".into());
        deck.add_card("f".into(), "b".into());
        app.storage.save_deck(&deck).unwrap();
        app.refresh_deck_list();

        app.screen = Screen::DeckSelect;
        let content = draw_at(&mut app, 80, 24);

        assert!(content.contains("SRL"), "header title rendered");
        assert!(content.contains("UI Deck"), "deck name rendered");
        assert!(content.contains("new/day"), "new/day setting surfaced");
    }

    #[test]
    fn study_header_shows_progress_bar() {
        let mut app = make_app();
        let mut deck = Deck::new("Prog".into());
        deck.add_card("q1".into(), "a1".into());
        deck.add_card("q2".into(), "a2".into());
        app.storage.save_deck(&deck).unwrap();
        app.refresh_deck_list();

        app.select_deck(&deck.id);
        let content = draw_at(&mut app, 80, 24);

        assert!(content.contains("Prog"), "deck name in header");
        assert!(content.contains("▱"), "progress bar rendered");
        assert!(content.contains("/2"), "session total rendered");
    }

    #[test]
    fn config_respects_new_per_session_limit() {
        let mut app = make_app();
        let mut deck = Deck::new("Limit".into());
        for i in 0..30 {
            deck.add_card(format!("q{i}"), format!("a{i}"));
        }
        app.storage.save_deck(&deck).unwrap();
        app.refresh_deck_list();

        app.config.new_per_session = 5;
        app.select_deck(&deck.id);
        // One card is in hand (next_card pops the queue head)…
        let in_hand = usize::from(app.current_card_idx.is_some());
        assert_eq!(app.study_queue.len() + in_hand, 5);

        app.config.new_per_session = 100;
        app.select_deck(&deck.id);
        let in_hand = usize::from(app.current_card_idx.is_some());
        assert_eq!(app.study_queue.len() + in_hand, 30);
    }
}
