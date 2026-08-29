//! Data models for flashcards and decks.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Rating for how well you remembered a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewRating {
    Again = 0, // Complete blackout
    Hard = 1,  // Serious difficulty
    Good = 2,  // Some hesitation
    Easy = 3,  // Perfect recall
}

impl ReviewRating {
    pub fn from_key(c: char) -> Option<Self> {
        match c {
            '1' => Some(Self::Again),
            '2' => Some(Self::Hard),
            '3' => Some(Self::Good),
            '4' => Some(Self::Easy),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Again => "Again",
            Self::Hard => "Hard",
            Self::Good => "Good",
            Self::Easy => "Easy",
        }
    }

    pub fn color_for_theme(&self, theme: &crate::ui::theme::Theme) -> ratatui::style::Color {
        match self {
            Self::Again => theme.colors.rating_again,
            Self::Hard => theme.colors.rating_hard,
            Self::Good => theme.colors.rating_good,
            Self::Easy => theme.colors.rating_easy,
        }
    }
}

/// A single flashcard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub front: String,
    pub back: String,

    // SM-2 fields
    pub ease_factor: f64,
    pub interval: u32,
    pub repetitions: u32,

    // Tracking
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<DateTime<Local>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reviewed: Option<DateTime<Local>>,
    pub total_reviews: u32,
    pub lapses: u32,

    // Metadata
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    pub created_at: DateTime<Local>,
}

impl Card {
    pub fn new(front: String, back: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            front,
            back,
            ease_factor: 2.5,
            interval: 0,
            repetitions: 0,
            due_date: None,
            last_reviewed: None,
            total_reviews: 0,
            lapses: 0,
            tags: Vec::new(),
            notes: String::new(),
            created_at: Local::now(),
        }
    }

    /// A card is "new" only if it has never been studied. A lapsed card also
    /// has `repetitions == 0` (SM-2 resets it on failure), but it must not be
    /// counted as new or re-queued through the new-card batch.
    pub fn is_new(&self) -> bool {
        self.repetitions == 0 && self.total_reviews == 0
    }

    pub fn is_due(&self) -> bool {
        match self.due_date {
            None => true,
            Some(due) => Local::now() >= due,
        }
    }

    /// Reset card to fresh/unlearned state.
    pub fn reset_progress(&mut self) {
        self.ease_factor = 2.5;
        self.interval = 0;
        self.repetitions = 0;
        self.due_date = None;
        self.last_reviewed = None;
        self.total_reviews = 0;
        self.lapses = 0;
    }
}

/// Statistics for a deck.
#[derive(Debug, Default)]
pub struct DeckStats {
    pub total_cards: usize,
    pub new_cards: usize,
    pub due_cards: usize,
    pub learning_cards: usize,
    pub mature_cards: usize,
}

/// A collection of flashcards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deck {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub cards: Vec<Card>,
    pub created_at: DateTime<Local>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_studied: Option<DateTime<Local>>,
}

impl Deck {
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            name,
            description: String::new(),
            cards: Vec::new(),
            created_at: Local::now(),
            last_studied: None,
        }
    }

    pub fn add_card(&mut self, front: String, back: String) -> &mut Card {
        let card = Card::new(front, back);
        self.cards.push(card);
        self.cards.last_mut().unwrap()
    }

    pub fn get_stats(&self) -> DeckStats {
        let mut stats = DeckStats {
            total_cards: self.cards.len(),
            ..Default::default()
        };

        for card in &self.cards {
            if card.is_new() {
                stats.new_cards += 1;
            } else if card.is_due() {
                stats.due_cards += 1;
            }

            if card.interval < 21 && !card.is_new() {
                stats.learning_cards += 1;
            } else if card.interval >= 21 {
                stats.mature_cards += 1;
            }
        }

        stats
    }

    /// Update a card's front and back text.
    pub fn update_card(&mut self, card_id: &str, front: String, back: String) -> bool {
        if let Some(card) = self.cards.iter_mut().find(|c| c.id == card_id) {
            card.front = front;
            card.back = back;
            true
        } else {
            false
        }
    }

    /// Delete a card by ID.
    pub fn delete_card(&mut self, card_id: &str) -> bool {
        let len_before = self.cards.len();
        self.cards.retain(|c| c.id != card_id);
        self.cards.len() < len_before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn only_never_studied_cards_are_new() {
        let mut c = Card::new("f".into(), "b".into());
        assert!(c.is_new());

        // Lapsed card: SM-2 resets repetitions to 0 on failure, but the card
        // has review history and must not be counted as new.
        c.total_reviews = 3;
        c.repetitions = 0;
        assert!(!c.is_new());
    }

    #[test]
    fn card_without_due_date_is_due() {
        let c = Card::new("f".into(), "b".into());
        assert!(c.is_due());
    }

    #[test]
    fn stats_counting() {
        let mut deck = Deck::new("d".into());

        // New card
        deck.add_card("q1".into(), "a1".into());

        // Mature card, not due
        let mature = deck.add_card("q2".into(), "a2".into());
        mature.interval = 30;
        mature.repetitions = 2;
        mature.total_reviews = 5;
        mature.due_date = Some(Local::now() + Duration::days(30));
        mature.last_reviewed = Some(Local::now() - Duration::days(1));

        // Learning card, overdue
        let learning = deck.add_card("q3".into(), "a3".into());
        learning.interval = 5;
        learning.repetitions = 1;
        learning.total_reviews = 2;
        learning.due_date = Some(Local::now() - Duration::days(1));
        learning.last_reviewed = Some(Local::now() - Duration::days(6));

        let stats = deck.get_stats();
        assert_eq!(stats.total_cards, 3);
        assert_eq!(stats.new_cards, 1);
        assert_eq!(stats.due_cards, 1);
        assert_eq!(stats.learning_cards, 1);
        assert_eq!(stats.mature_cards, 1);
    }

    #[test]
    fn update_and_delete_card_by_id() {
        let mut deck = Deck::new("d".into());
        let c = deck.add_card("q".into(), "a".into());
        let id = c.id.clone();

        assert!(deck.update_card(&id, "q2".into(), "a2".into()));
        assert_eq!(deck.cards[0].front, "q2");

        assert!(deck.delete_card(&id));
        assert!(deck.cards.is_empty());
        assert!(!deck.delete_card(&id));
    }

    #[test]
    fn reset_progress_restores_fresh_state() {
        let mut c = Card::new("f".into(), "b".into());
        c.interval = 42;
        c.repetitions = 5;
        c.ease_factor = 1.9;
        c.total_reviews = 10;
        c.lapses = 2;
        c.due_date = Some(Local::now());
        c.reset_progress();

        assert!(c.is_new());
        assert!(c.is_due());
        assert_eq!(c.interval, 0);
        assert_eq!(c.ease_factor, 2.5);
        assert_eq!(c.total_reviews, 0);
        assert_eq!(c.lapses, 0);
    }
}
