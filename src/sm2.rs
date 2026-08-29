//! SM-2 Spaced Repetition Algorithm
//!
//! Implementation of the SuperMemo SM-2 algorithm for scheduling flashcard reviews.

use chrono::{Duration, Local};

use crate::models::{Card, ReviewRating};

/// SM-2 scheduler for flashcard reviews.
pub struct Scheduler {
    min_ease: f64,
    easy_bonus: f64,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            min_ease: 1.3,
            easy_bonus: 1.3,
        }
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate new ease factor based on rating.
    fn calculate_ease_factor(&self, card: &Card, rating: ReviewRating) -> f64 {
        // Map 0-3 rating to SM-2's 0-5 scale
        let q = (rating as u8) as f64 * 5.0 / 3.0;

        // SM-2 formula
        let new_ef = card.ease_factor + (0.1 - (5.0 - q) * (0.08 + (5.0 - q) * 0.02));

        new_ef.max(self.min_ease)
    }

    /// Calculate the next review interval in days.
    fn calculate_interval(&self, card: &Card, rating: ReviewRating, new_ease: f64) -> u32 {
        if rating == ReviewRating::Again {
            return 1;
        }

        if card.repetitions == 0 {
            // First review
            return match rating {
                ReviewRating::Again => 0,
                ReviewRating::Hard => 1,
                ReviewRating::Good => 1,
                ReviewRating::Easy => 4,
            };
        }

        // Subsequent reviews (including the second one) multiply the current
        // interval, matching the documented "Good = interval × EF" contract.
        let current = card.interval.max(1) as f64;

        let new_interval = match rating {
            ReviewRating::Again => 1.0,
            ReviewRating::Hard => current * 1.2,
            ReviewRating::Good => current * new_ease,
            ReviewRating::Easy => current * new_ease * self.easy_bonus,
        };

        (new_interval.round() as u32).max(1)
    }

    /// Process a card review and update its state.
    pub fn review_card(&self, card: &mut Card, rating: ReviewRating) {
        let now = Local::now();

        // Calculate new values
        let new_ease_factor = self.calculate_ease_factor(card, rating);
        let new_interval = self.calculate_interval(card, rating, new_ease_factor);

        // Update repetitions
        let new_repetitions = if rating == ReviewRating::Again {
            card.lapses += 1;
            0
        } else {
            card.repetitions + 1
        };

        // Calculate next due date
        let next_due = if rating == ReviewRating::Again && card.is_new() {
            now + Duration::minutes(10)
        } else {
            now + Duration::days(new_interval as i64)
        };

        // Update card
        card.ease_factor = new_ease_factor;
        card.interval = new_interval;
        card.repetitions = new_repetitions;
        card.due_date = Some(next_due);
        card.last_reviewed = Some(now);
        card.total_reviews += 1;
    }

    /// Get human-readable interval string.
    pub fn interval_string(days: u32) -> String {
        if days == 0 {
            "< 1 min".to_string()
        } else if days == 1 {
            "1 day".to_string()
        } else if days < 7 {
            format!("{} days", days)
        } else if days < 30 {
            let weeks = days / 7;
            format!("{} week{}", weeks, if weeks > 1 { "s" } else { "" })
        } else if days < 365 {
            let months = days / 30;
            format!("{} month{}", months, if months > 1 { "s" } else { "" })
        } else {
            let years = days / 365;
            format!("{} year{}", years, if years > 1 { "s" } else { "" })
        }
    }

    /// Preview intervals for each rating.
    pub fn preview_intervals(&self, card: &Card) -> [(ReviewRating, String); 4] {
        let ratings = [
            ReviewRating::Again,
            ReviewRating::Hard,
            ReviewRating::Good,
            ReviewRating::Easy,
        ];

        ratings.map(|rating| {
            let new_ef = self.calculate_ease_factor(card, rating);
            let interval = self.calculate_interval(card, rating, new_ef);

            let interval_str = if rating == ReviewRating::Again && card.is_new() {
                "10 min".to_string()
            } else {
                Self::interval_string(interval)
            };

            (rating, interval_str)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Card;

    fn card(interval: u32, repetitions: u32, ease: f64) -> Card {
        let mut c = Card::new("front".into(), "back".into());
        c.interval = interval;
        c.repetitions = repetitions;
        c.ease_factor = ease;
        c
    }

    #[test]
    fn first_review_intervals() {
        let s = Scheduler::new();
        assert_eq!(
            s.calculate_interval(&card(0, 0, 2.5), ReviewRating::Good, 2.5),
            1
        );
        assert_eq!(
            s.calculate_interval(&card(0, 0, 2.5), ReviewRating::Hard, 2.11),
            1
        );
        assert_eq!(
            s.calculate_interval(&card(0, 0, 2.5), ReviewRating::Easy, 2.6),
            4
        );
        assert_eq!(
            s.calculate_interval(&card(0, 0, 2.5), ReviewRating::Again, 1.7),
            1
        );
    }

    #[test]
    fn subsequent_reviews_multiply_interval() {
        let s = Scheduler::new();
        // Second review: interval 1 × EF(2.41) → 2 (README contract: Good = interval × EF)
        assert_eq!(
            s.calculate_interval(&card(1, 1, 2.41), ReviewRating::Good, 2.41),
            2
        );
        // Then 2 × 2.41 → 5
        assert_eq!(
            s.calculate_interval(&card(2, 2, 2.41), ReviewRating::Good, 2.41),
            5
        );
        // Hard multiplies by 1.2
        assert_eq!(
            s.calculate_interval(&card(10, 3, 2.5), ReviewRating::Hard, 2.5),
            12
        );
        // Easy multiplies by EF × easy_bonus(1.3)
        assert_eq!(
            s.calculate_interval(&card(10, 3, 2.5), ReviewRating::Easy, 2.6),
            34
        );
    }

    #[test]
    fn ease_factor_updates_and_floor() {
        let s = Scheduler::new();
        // q=0 (Again): EF -0.8
        assert!(
            (s.calculate_ease_factor(&card(0, 0, 2.5), ReviewRating::Again) - 1.7).abs() < 1e-9
        );
        // q=5 (Easy): EF +0.1
        assert!((s.calculate_ease_factor(&card(0, 0, 2.5), ReviewRating::Easy) - 2.6).abs() < 1e-9);
        // Repeated failures clamp at min_ease
        let mut c = card(0, 0, 1.4);
        s.review_card(&mut c, ReviewRating::Again);
        assert!((c.ease_factor - 1.3).abs() < 1e-9);
    }

    #[test]
    fn again_on_fresh_card_is_a_10_minute_relearn() {
        let s = Scheduler::new();
        let mut c = Card::new("f".into(), "b".into());
        s.review_card(&mut c, ReviewRating::Again);
        assert_eq!(c.repetitions, 0);
        assert_eq!(c.total_reviews, 1);
        assert_eq!(c.lapses, 1);
        let secs = (c.due_date.unwrap() - Local::now()).num_seconds();
        assert!(secs > 0 && secs <= 600, "due in ~10 minutes, got {}s", secs);
    }

    #[test]
    fn again_on_studied_card_is_one_day() {
        let s = Scheduler::new();
        let mut c = card(15, 3, 2.5);
        s.review_card(&mut c, ReviewRating::Again);
        assert_eq!(c.repetitions, 0);
        assert_eq!(c.interval, 1);
        // The card now has review history, so due date is ~1 day out
        let hours = (c.due_date.unwrap() - Local::now()).num_hours();
        assert!((23..=24).contains(&hours), "due in ~1 day, got {}h", hours);
    }

    #[test]
    fn good_sequence_grows_intervals() {
        let s = Scheduler::new();
        let mut c = Card::new("f".into(), "b".into());
        s.review_card(&mut c, ReviewRating::Good);
        assert_eq!(c.interval, 1);
        s.review_card(&mut c, ReviewRating::Good);
        assert_eq!(c.interval, 2);
        s.review_card(&mut c, ReviewRating::Good);
        // EF after two Goods: 2.5 → 2.4111 → 2.3222; 2 × 2.3222 = 4.4667 → 4
        assert_eq!(c.interval, 4);
    }

    #[test]
    fn preview_labels() {
        let s = Scheduler::new();
        let preview = s.preview_intervals(&Card::new("f".into(), "b".into()));
        assert_eq!(preview[0].1, "10 min"); // Again on a fresh card
        assert_eq!(preview[3].1, "4 days"); // Easy first review
    }

    #[test]
    fn interval_string() {
        assert_eq!(Scheduler::interval_string(0), "< 1 min");
        assert_eq!(Scheduler::interval_string(1), "1 day");
        assert_eq!(Scheduler::interval_string(5), "5 days");
        assert_eq!(Scheduler::interval_string(14), "2 weeks");
        assert_eq!(Scheduler::interval_string(60), "2 months");
        assert_eq!(Scheduler::interval_string(400), "1 year");
        assert_eq!(Scheduler::interval_string(800), "2 years");
    }
}
