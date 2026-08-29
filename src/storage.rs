//! Storage module for saving and loading flashcard decks.

use anyhow::{Context, Result};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::models::{Card, Deck};

/// Bundled deck: Development Workflow
const BUNDLED_DEV_WORKFLOW: &str = include_str!("../bundled_decks/development-workflow.json");

/// Handles deck persistence.
pub struct DeckStorage {
    decks_dir: PathBuf,
}

impl DeckStorage {
    pub fn new(decks_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&decks_dir)
            .with_context(|| format!("Failed to create decks directory: {:?}", decks_dir))?;

        let storage = Self { decks_dir };
        storage.install_bundled_decks();
        Ok(storage)
    }

    /// Install bundled decks if they don't already exist.
    fn install_bundled_decks(&self) {
        // Check if any decks exist - if so, user has already used the app
        if let Ok(entries) = fs::read_dir(&self.decks_dir) {
            if entries
                .filter_map(|e| e.ok())
                .any(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            {
                return; // User already has decks, don't overwrite
            }
        }

        // Install bundled decks for first-time users
        if let Ok(mut deck) = serde_json::from_str::<Deck>(BUNDLED_DEV_WORKFLOW) {
            // Reset all cards to fresh state
            for card in &mut deck.cards {
                card.reset_progress();
            }
            let _ = self.save_deck(&deck);
        }
    }

    /// Get default storage location.
    pub fn default_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("flashcards")
            .join("decks")
    }

    fn deck_path(&self, deck_id: &str) -> PathBuf {
        self.decks_dir.join(format!("{}.json", deck_id))
    }

    /// Save a deck to disk (atomic: write to a temp file, then rename over
    /// the destination so a crash can never leave a half-written JSON).
    pub fn save_deck(&self, deck: &Deck) -> Result<PathBuf> {
        let path = self.deck_path(&deck.id);
        let tmp_path = self.decks_dir.join(format!("{}.json.tmp", deck.id));
        let json = serde_json::to_string_pretty(deck)?;
        fs::write(&tmp_path, json)?;
        fs::rename(&tmp_path, &path)
            .with_context(|| format!("Failed to replace deck file: {:?}", path))?;
        Ok(path)
    }

    /// Load a deck from disk.
    pub fn load_deck(&self, deck_id: &str) -> Result<Option<Deck>> {
        let path = self.deck_path(deck_id);
        if !path.exists() {
            return Ok(None);
        }

        let json = fs::read_to_string(&path)?;
        let deck: Deck = serde_json::from_str(&json)?;
        Ok(Some(deck))
    }

    /// Delete a deck file.
    pub fn delete_deck(&self, deck_id: &str) -> Result<bool> {
        let path = self.deck_path(deck_id);
        if path.exists() {
            fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// List all available decks.
    pub fn list_decks(&self) -> Result<Vec<DeckInfo>> {
        let mut decks = Vec::new();

        for entry in fs::read_dir(&self.decks_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|e| e == "json") {
                if let Ok(json) = fs::read_to_string(&path) {
                    if let Ok(deck) = serde_json::from_str::<Deck>(&json) {
                        decks.push(DeckInfo {
                            id: deck.id,
                            name: deck.name,
                            card_count: deck.cards.len(),
                        });
                    }
                }
            }
        }

        decks.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(decks)
    }

    /// Import cards from a CSV file.
    /// Format: front,back[,tags] — a leading header row is skipped only when
    /// its first cell is literally "front" (case-insensitive).
    pub fn import_csv(&self, csv_path: &Path, deck_name: &str) -> Result<Deck> {
        let mut deck = Deck::new(deck_name.to_string());
        let content = fs::read_to_string(csv_path)?;

        for (i, line) in content
            .strip_prefix('\u{feff}')
            .unwrap_or(&content)
            .lines()
            .enumerate()
        {
            let parts = parse_csv_line(line);

            // Skip header row (only when the first cell is the literal "front")
            if i == 0
                && parts
                    .first()
                    .map(|f| f.trim().eq_ignore_ascii_case("front"))
                    .unwrap_or(false)
            {
                continue;
            }

            if parts.len() >= 2 {
                let front = parts[0].trim().to_string();
                let back = parts[1].trim().to_string();

                if !front.is_empty() && !back.is_empty() {
                    let card = deck.add_card(front, back);

                    // Optional third column: tags (space separated)
                    if parts.len() >= 3 {
                        let tags: Vec<String> =
                            parts[2].split_whitespace().map(|t| t.to_string()).collect();
                        card.tags = tags;
                    }
                }
            }
        }

        Ok(deck)
    }

    /// Import all CSV files from a folder.
    /// Names decks based on filename, converting snake_case/kebab-case to Title Case.
    /// Skips any deck whose name already exists.
    /// Returns (imported, skipped) tuple.
    pub fn import_folder(&self, folder_path: &Path) -> Result<FolderImportResult> {
        let mut imported = Vec::new();
        let mut skipped = Vec::new();

        // Get existing deck names for duplicate check
        let existing_names: std::collections::HashSet<String> = self
            .list_decks()?
            .into_iter()
            .map(|d| d.name.to_lowercase())
            .collect();

        for entry in fs::read_dir(folder_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|e| e == "csv") {
                let deck_name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(filename_to_title_case)
                    .unwrap_or_else(|| "Imported Deck".to_string());

                // Skip if deck with this name already exists
                if existing_names.contains(&deck_name.to_lowercase()) {
                    skipped.push(deck_name);
                    continue;
                }

                match self.import_csv(&path, &deck_name) {
                    Ok(deck) => {
                        let card_count = deck.cards.len();
                        if card_count > 0 {
                            self.save_deck(&deck)?;
                            imported.push((deck_name, card_count));
                        }
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to import {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok((imported, skipped))
    }

    /// Check if a deck with the given name already exists.
    pub fn deck_name_exists(&self, name: &str) -> bool {
        self.list_decks()
            .map(|decks| {
                decks
                    .iter()
                    .any(|d| d.name.to_lowercase() == name.to_lowercase())
            })
            .unwrap_or(false)
    }

    /// Import cards from an Anki text export (tab-separated or semicolon-separated).
    /// Format: front<TAB>back or front;back, with optional tags column.
    pub fn import_anki_text(&self, path: &Path, deck_name: &str) -> Result<Deck> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed to read Anki text file: {:?}", path))?;
        // Strip a UTF-8 BOM if present (common in Windows-exported files)
        let content = raw.strip_prefix('\u{feff}').unwrap_or(&raw);

        let mut deck = Deck::new(deck_name.to_string());

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Detect delimiter: tab or semicolon
            let parts: Vec<&str> = if line.contains('\t') {
                line.split('\t').collect()
            } else {
                line.split(';').collect()
            };

            if parts.len() >= 2 {
                let front = parts[0].trim().to_string();
                let back = parts[1].trim().to_string();

                if !front.is_empty() && !back.is_empty() {
                    let mut card = Card::new(front, back);

                    // If there's a third column, treat it as tags
                    if parts.len() >= 3 {
                        let tags: Vec<String> =
                            parts[2].split_whitespace().map(|t| t.to_string()).collect();
                        card.tags = tags;
                    }

                    deck.cards.push(card);
                }
            }
        }

        Ok(deck)
    }

    /// Import a deck from an Anki .apkg package file.
    /// APKG files are ZIP archives containing a SQLite database.
    pub fn import_apkg(&self, path: &Path) -> Result<Vec<Deck>> {
        use rusqlite::Connection;
        use zip::ZipArchive;

        let file =
            File::open(path).with_context(|| format!("Failed to open APKG file: {:?}", path))?;

        let mut archive =
            ZipArchive::new(file).with_context(|| "Failed to read APKG as ZIP archive")?;

        // Find and extract the SQLite database
        // Anki 2.1+ uses collection.anki21, older versions use collection.anki2
        let db_name = if archive.file_names().any(|n| n == "collection.anki21") {
            "collection.anki21"
        } else if archive.file_names().any(|n| n == "collection.anki2") {
            "collection.anki2"
        } else {
            anyhow::bail!("No Anki database found in APKG file (expected collection.anki21 or collection.anki2)");
        };

        // Extract database to a temporary file
        let mut db_file = archive
            .by_name(db_name)
            .with_context(|| format!("Failed to extract {} from APKG", db_name))?;

        let temp_dir = std::env::temp_dir();
        let temp_db_path = temp_dir.join(format!("anki_import_{}.db", uuid::Uuid::new_v4()));

        let mut temp_file = File::create(&temp_db_path)
            .with_context(|| "Failed to create temporary database file")?;
        std::io::copy(&mut db_file, &mut temp_file)
            .with_context(|| "Failed to extract database")?;
        drop(temp_file);

        // Open the SQLite database
        let conn =
            Connection::open(&temp_db_path).with_context(|| "Failed to open Anki database")?;

        // Collection creation date (UTC seconds) — Anki review-card due dates
        // are day offsets counted from this date.
        let crt_secs: i64 = {
            let mut stmt = conn.prepare("SELECT crt FROM col")?;
            stmt.query_row([], |row| row.get(0))
                .with_context(|| "Failed to read collection creation date")?
        };
        let crt_date = chrono::DateTime::from_timestamp(crt_secs, 0)
            .map(|dt| dt.with_timezone(&chrono::Local).date_naive())
            .unwrap_or_else(|| chrono::Local::now().date_naive());
        let today = chrono::Local::now().date_naive();
        let days_since_crt = (today - crt_date).num_days();

        // Get deck names from the col table
        let deck_names: std::collections::HashMap<i64, String> = {
            let mut stmt = conn.prepare("SELECT decks FROM col")?;
            let decks_json: String = stmt.query_row([], |row| row.get(0))?;
            let decks: serde_json::Value = serde_json::from_str(&decks_json)?;

            decks
                .as_object()
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(id, info)| {
                            let deck_id: i64 = id.parse().ok()?;
                            let name = info.get("name")?.as_str()?.to_string();
                            Some((deck_id, name))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };

        // Query notes and cards with scheduling info.
        // Join notes (content) with cards (scheduling, deck assignment) and
        // keep only the first template (c.ord = 0) so multi-template note
        // types (e.g. "Basic (and reversed card)") don't import duplicates.
        let mut stmt = conn.prepare(
            "SELECT n.flds, n.tags, c.did, c.type, c.due, c.ivl, c.factor, c.reps, c.lapses
             FROM notes n
             JOIN cards c ON c.nid = n.id
             WHERE c.ord = 0",
        )?;

        // Group cards by deck
        let mut decks_map: std::collections::HashMap<i64, Vec<Card>> =
            std::collections::HashMap::new();

        let rows = stmt.query_map([], |row| {
            let flds: String = row.get(0)?;
            let tags: String = row.get(1)?;
            let did: i64 = row.get(2)?;
            let card_type: i32 = row.get(3)?;
            let due: i64 = row.get(4)?;
            let ivl: i32 = row.get(5)?;
            let factor: i32 = row.get(6)?;
            let reps: i32 = row.get(7)?;
            let lapses: i32 = row.get(8)?;
            Ok((flds, tags, did, card_type, due, ivl, factor, reps, lapses))
        })?;

        for row in rows {
            let (flds, tags, did, card_type, due, ivl, factor, reps, lapses) = row?;

            // Split fields by Anki's field separator (0x1f)
            let fields: Vec<&str> = flds.split('\x1f').collect();
            if fields.is_empty() {
                continue;
            }

            // Field layout varies by note type: "Basic" is front=fields[0],
            // back=fields[1], but multi-field templates (e.g. 成语 decks) keep
            // the meaning in a later field, and cloze notes embed everything
            // in fields[0]. Derive front/back accordingly instead of dying on
            // an empty fields[1].
            let (front, back) = if fields[0].contains("{{c") {
                let answers = extract_cloze_answers(fields[0]);
                let rest: Vec<&str> = fields[1..]
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                let mut b = String::new();
                if !answers.is_empty() {
                    b.push_str(&format!("答案：{answers}"));
                }
                if !rest.is_empty() {
                    if !b.is_empty() {
                        b.push('\n');
                    }
                    b.push_str(&strip_html(&rest.join("\n")));
                }
                (strip_html(&cloze_blank(fields[0])), b)
            } else {
                let f = strip_html(fields[0]);
                let b = fields[1..]
                    .iter()
                    .map(|s| s.trim())
                    .find(|s| !s.is_empty())
                    .map(strip_html)
                    .unwrap_or_default();
                (f, b)
            };

            if front.is_empty() || back.is_empty() {
                continue;
            }

            // Create card with imported scheduling data
            let mut card = Card::new(front, back);
            card.tags = tags.split_whitespace().map(|t| t.to_string()).collect();
            card.interval = ivl.max(0) as u32;
            // New/never-reviewed Anki cards store factor = 0, which would
            // import as ease 0.00; fall back to the SM-2 default instead.
            card.ease_factor = if factor >= 1300 {
                (factor as f64) / 1000.0
            } else {
                2.5
            };
            card.repetitions = reps.max(0) as u32;
            card.lapses = lapses.max(0) as u32;

            // Restore the due date according to Anki's per-type semantics:
            //   type 0 = new          -> no due date (new card)
            //   type 1/3 = (re)learning -> due is an epoch-seconds timestamp
            //   type 2 = review       -> due is a day offset since col.crt
            match card_type {
                2 => {
                    let days_until = due - days_since_crt;
                    card.due_date = Some(chrono::Local::now() + chrono::Duration::days(days_until));
                }
                1 | 3 => {
                    card.due_date = chrono::DateTime::from_timestamp(due.max(0), 0)
                        .map(|dt| dt.with_timezone(&chrono::Local));
                }
                _ => {}
            }

            decks_map.entry(did).or_default().push(card);
        }

        // Clean up temp file
        let _ = fs::remove_file(&temp_db_path);

        // Create Deck objects
        let mut result = Vec::new();
        for (did, cards) in decks_map {
            let name = deck_names
                .get(&did)
                .cloned()
                .unwrap_or_else(|| format!("Imported Deck {}", did));

            let mut deck = Deck::new(name);
            deck.cards = cards;
            result.push(deck);
        }

        if result.is_empty() {
            anyhow::bail!("No cards found in APKG file");
        }

        Ok(result)
    }

    /// Export decks to an Anki .apkg package file.
    /// Preserves scheduling data (interval, ease factor, repetitions, lapses).
    pub fn export_apkg(&self, path: &Path, deck_ids: Option<&[String]>) -> Result<usize> {
        use rusqlite::Connection;
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        // Load decks to export
        let deck_infos = self.list_decks()?;
        let decks_to_export: Vec<Deck> = if let Some(ids) = deck_ids {
            ids.iter()
                .filter_map(|id| self.load_deck(id).ok().flatten())
                .collect()
        } else {
            deck_infos
                .iter()
                .filter_map(|info| self.load_deck(&info.id).ok().flatten())
                .collect()
        };

        if decks_to_export.is_empty() {
            anyhow::bail!("No decks to export");
        }

        // Create temporary SQLite database
        let temp_dir = std::env::temp_dir();
        let temp_db_path = temp_dir.join(format!("anki_export_{}.db", uuid::Uuid::new_v4()));
        let conn = Connection::open(&temp_db_path)
            .with_context(|| "Failed to create temporary database")?;

        // Create Anki schema
        conn.execute_batch(
            r#"
            CREATE TABLE col (
                id INTEGER PRIMARY KEY,
                crt INTEGER NOT NULL,
                mod INTEGER NOT NULL,
                scm INTEGER NOT NULL,
                ver INTEGER NOT NULL,
                dty INTEGER NOT NULL,
                usn INTEGER NOT NULL,
                ls INTEGER NOT NULL,
                conf TEXT NOT NULL,
                models TEXT NOT NULL,
                decks TEXT NOT NULL,
                dconf TEXT NOT NULL,
                tags TEXT NOT NULL
            );
            CREATE TABLE notes (
                id INTEGER PRIMARY KEY,
                guid TEXT NOT NULL,
                mid INTEGER NOT NULL,
                mod INTEGER NOT NULL,
                usn INTEGER NOT NULL,
                tags TEXT NOT NULL,
                flds TEXT NOT NULL,
                sfld TEXT NOT NULL,
                csum INTEGER NOT NULL,
                flags INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            CREATE TABLE cards (
                id INTEGER PRIMARY KEY,
                nid INTEGER NOT NULL,
                did INTEGER NOT NULL,
                ord INTEGER NOT NULL,
                mod INTEGER NOT NULL,
                usn INTEGER NOT NULL,
                type INTEGER NOT NULL,
                queue INTEGER NOT NULL,
                due INTEGER NOT NULL,
                ivl INTEGER NOT NULL,
                factor INTEGER NOT NULL,
                reps INTEGER NOT NULL,
                lapses INTEGER NOT NULL,
                left INTEGER NOT NULL,
                odue INTEGER NOT NULL,
                odid INTEGER NOT NULL,
                flags INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            CREATE TABLE revlog (
                id INTEGER PRIMARY KEY,
                cid INTEGER NOT NULL,
                usn INTEGER NOT NULL,
                ease INTEGER NOT NULL,
                ivl INTEGER NOT NULL,
                lastIvl INTEGER NOT NULL,
                factor INTEGER NOT NULL,
                time INTEGER NOT NULL,
                type INTEGER NOT NULL
            );
            CREATE TABLE graves (
                usn INTEGER NOT NULL,
                oid INTEGER NOT NULL,
                type INTEGER NOT NULL
            );
            "#,
        )?;

        let now = chrono::Utc::now().timestamp();
        let now_millis = now * 1000;
        // Anki convention: col.crt is the LOCAL midnight of the collection
        // creation date, expressed as UTC seconds. Review-card due dates are
        // day offsets from it, so using "now" here would shift every due
        // date by up to a day across timezones.
        let crt = chrono::Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .earliest()
            .map(|dt| dt.timestamp())
            .unwrap_or(now);

        // Build deck JSON for col table
        let mut decks_json = serde_json::Map::new();
        // Default deck (id=1)
        decks_json.insert(
            "1".to_string(),
            serde_json::json!({
                "id": 1,
                "name": "Default",
                "mod": now,
                "usn": -1,
                "lrnToday": [0, 0],
                "revToday": [0, 0],
                "newToday": [0, 0],
                "timeToday": [0, 0],
                "collapsed": false,
                "desc": "",
                "dyn": 0,
                "conf": 1,
                "extendNew": 10,
                "extendRev": 50
            }),
        );

        // Add our decks
        for (i, deck) in decks_to_export.iter().enumerate() {
            let deck_id = (i as i64 + 2) * 1000000000000i64 + 1;
            decks_json.insert(
                deck_id.to_string(),
                serde_json::json!({
                    "id": deck_id,
                    "name": deck.name,
                    "mod": now,
                    "usn": -1,
                    "lrnToday": [0, 0],
                    "revToday": [0, 0],
                    "newToday": [0, 0],
                    "timeToday": [0, 0],
                    "collapsed": false,
                    "desc": deck.description,
                    "dyn": 0,
                    "conf": 1,
                    "extendNew": 10,
                    "extendRev": 50
                }),
            );
        }

        // Basic model (note type) for simple front/back cards
        let model_id: i64 = 1000000000001;
        let models_json = serde_json::json!({
            model_id.to_string(): {
                "id": model_id,
                "name": "Basic",
                "type": 0,
                "mod": now,
                "usn": -1,
                "sortf": 0,
                "did": 1,
                "tmpls": [{
                    "name": "Card 1",
                    "ord": 0,
                    "qfmt": "{{Front}}",
                    "afmt": "{{FrontSide}}<hr id=answer>{{Back}}",
                    "did": null,
                    "bqfmt": "",
                    "bafmt": ""
                }],
                "flds": [
                    {"name": "Front", "ord": 0, "sticky": false, "rtl": false, "font": "Arial", "size": 20, "media": []},
                    {"name": "Back", "ord": 1, "sticky": false, "rtl": false, "font": "Arial", "size": 20, "media": []}
                ],
                "css": ".card { font-family: arial; font-size: 20px; text-align: center; color: black; background-color: white; }",
                "latexPre": "",
                "latexPost": "",
                "latexsvg": false,
                "req": [[0, "all", [0]]]
            }
        });

        // Default deck config
        let dconf_json = serde_json::json!({
            "1": {
                "id": 1,
                "name": "Default",
                "replayq": true,
                "lapse": {"leechFails": 8, "minInt": 1, "delays": [10], "leechAction": 0, "mult": 0},
                "rev": {"perDay": 200, "fuzz": 0.05, "ivlFct": 1, "maxIvl": 36500, "ease4": 1.3, "bury": false, "hardFactor": 1.2},
                "new": {"perDay": 20, "delays": [1, 10], "separate": true, "ints": [1, 4, 7], "initialFactor": 2500, "bury": false, "order": 1},
                "maxTaken": 60,
                "timer": 0,
                "autoplay": true,
                "mod": 0,
                "usn": 0
            }
        });

        // Insert collection metadata
        conn.execute(
            "INSERT INTO col VALUES (1, ?, ?, ?, 11, 0, -1, 0, '{}', ?, ?, ?, '{}')",
            rusqlite::params![
                crt,
                now,
                now_millis,
                models_json.to_string(),
                serde_json::Value::Object(decks_json).to_string(),
                dconf_json.to_string(),
            ],
        )?;

        // Insert notes and cards
        let mut note_id: i64 = now_millis;
        let mut card_id: i64 = now_millis;
        let mut total_cards = 0;

        for (deck_idx, deck) in decks_to_export.iter().enumerate() {
            let deck_id = (deck_idx as i64 + 2) * 1000000000000i64 + 1;

            for card in &deck.cards {
                note_id += 1;
                card_id += 1;

                // Fields separated by 0x1f
                let flds = format!("{}\x1f{}", card.front, card.back);
                let tags = card.tags.join(" ");

                // Simple checksum of front field
                let csum: i64 = card.front.bytes().map(|b| b as i64).sum::<i64>() % 2147483647;

                // Insert note
                conn.execute(
                    "INSERT INTO notes VALUES (?, ?, ?, ?, -1, ?, ?, ?, ?, 0, '')",
                    rusqlite::params![
                        note_id,
                        &card.id, // guid
                        model_id,
                        now,
                        tags,
                        flds,
                        &card.front, // sfld (sort field)
                        csum,
                    ],
                )?;

                // Determine card type and queue
                let (card_type, queue, due) = if card.repetitions == 0 {
                    (0, 0, note_id) // New card
                } else if card.interval == 0 {
                    (1, 1, now) // Learning
                } else {
                    // Review card - due is a day offset counted from the
                    // collection creation date (crt = today), so convert the
                    // card's actual due date into that offset.
                    let due_days = card
                        .due_date
                        .map(|d| (d.date_naive() - chrono::Local::now().date_naive()).num_days())
                        .unwrap_or(card.interval as i64);
                    (2, 2, due_days)
                };

                // Insert card with scheduling data
                conn.execute(
                    "INSERT INTO cards VALUES (?, ?, ?, 0, ?, -1, ?, ?, ?, ?, ?, ?, ?, 0, 0, 0, 0, '')",
                    rusqlite::params![
                        card_id,
                        note_id,
                        deck_id,
                        now,
                        card_type,
                        queue,
                        due,
                        card.interval as i64,
                        (card.ease_factor * 1000.0) as i64,
                        card.repetitions as i64,
                        card.lapses as i64,
                    ],
                )?;

                total_cards += 1;
            }
        }

        conn.close().map_err(|(_, e)| e)?;

        // Create the APKG (ZIP) file
        let apkg_file = File::create(path)
            .with_context(|| format!("Failed to create APKG file: {:?}", path))?;
        let mut zip = ZipWriter::new(apkg_file);

        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        // Add the database
        zip.start_file("collection.anki2", options)?;
        let db_bytes = fs::read(&temp_db_path)?;
        zip.write_all(&db_bytes)?;

        // Add empty media file
        zip.start_file("media", options)?;
        zip.write_all(b"{}")?;

        zip.finish()?;

        // Clean up temp file
        let _ = fs::remove_file(&temp_db_path);

        Ok(total_cards)
    }

    /// Auto-detect Anki format and import.
    /// Returns the imported decks.
    pub fn import_anki(&self, path: &Path, deck_name: Option<&str>) -> Result<Vec<Deck>> {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        match extension.as_deref() {
            Some("apkg") => self.import_apkg(path),
            Some("txt") | Some("tsv") => {
                let name = deck_name.unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Imported Deck")
                });
                let deck = self.import_anki_text(path, name)?;
                Ok(vec![deck])
            }
            _ => {
                // Try to detect format from content
                let content = fs::read_to_string(path)?;
                if content.contains('\t') || content.contains(';') {
                    let name = deck_name.unwrap_or("Imported Deck");
                    let deck = self.import_anki_text(path, name)?;
                    Ok(vec![deck])
                } else {
                    anyhow::bail!("Unknown file format. Expected .apkg, .txt, or .tsv file.")
                }
            }
        }
    }
}

/// Parse a CSV line respecting quoted fields.
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' if !in_quotes => {
                in_quotes = true;
            }
            '"' if in_quotes => {
                // Check for escaped quote ("")
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            ',' if !in_quotes => {
                fields.push(current.clone());
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }
    fields.push(current);
    fields
}

/// Strip HTML tags from a string (basic implementation).
fn strip_html(s: &str) -> String {
    // Convert line-break and block-closing tags to newlines BEFORE stripping,
    // otherwise the tag stripper would swallow them and lose all line breaks.
    let lowered_breaks = s
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n");

    let mut result = String::new();
    let mut in_tag = false;

    for c in lowered_breaks.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    // Decode common HTML entities
    result
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

/// One `{{cN::text(::hint)?}}` occurrence inside a cloze note.
struct ClozeHit {
    start: usize,
    end: usize,
    n: u32,
    text: String,
    hint: Option<String>,
}

/// Find all cloze deletions in a note body (hand-rolled scan; no regex dep).
fn find_cloze_hits(s: &str) -> Vec<ClozeHit> {
    let bytes = s.as_bytes();
    let mut hits = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(rel) = s[i + 2..].find("}}") {
                let inner = &s[i + 2..i + 2 + rel];
                let after_c = inner.strip_prefix('c').unwrap_or("");
                let mut it = after_c.splitn(2, "::");
                let num = it.next().unwrap_or("");
                if let Ok(n) = num.parse::<u32>() {
                    let tail = it.next().unwrap_or("");
                    let mut parts = tail.splitn(2, "::");
                    let text = parts.next().unwrap_or("").trim().to_string();
                    let hint = parts
                        .next()
                        .map(|h| h.trim().to_string())
                        .filter(|h| !h.is_empty());
                    hits.push(ClozeHit {
                        start: i,
                        end: i + 2 + rel + 2,
                        n,
                        text,
                        hint,
                    });
                    i += 2 + rel + 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    hits
}

/// Blank out the `{{c1::…}}` deletions of a cloze note for use as a question
/// (hint kept in brackets); other cloze numbers render as plain text, matching
/// Anki's first-template rendering.
fn cloze_blank(s: &str) -> String {
    let mut out = String::new();
    let mut last = 0;
    for h in find_cloze_hits(s) {
        out.push_str(&s[last..h.start]);
        if h.n == 1 {
            match &h.hint {
                Some(hint) => out.push_str(&format!("［{hint}］")),
                None => out.push_str("＿＿＿"),
            }
        } else {
            out.push_str(&h.text);
        }
        last = h.end;
    }
    out.push_str(&s[last..]);
    out
}

/// Collect the answers of all `{{c1::…}}` deletions, joined by "；".
fn extract_cloze_answers(s: &str) -> String {
    let answers: Vec<String> = find_cloze_hits(s)
        .into_iter()
        .filter(|h| h.n == 1)
        .map(|h| h.text)
        .filter(|t| !t.is_empty())
        .collect();
    answers.join("；")
}

/// Convert a filename (snake_case or kebab-case) to Title Case.
fn filename_to_title_case(name: &str) -> String {
    name.split(['_', '-'])
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().collect::<String>()
                        + chars.as_str().to_lowercase().as_str()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Summary info for a deck.
#[derive(Debug, Clone)]
pub struct DeckInfo {
    pub id: String,
    pub name: String,
    pub card_count: usize,
}

/// Backup format containing all decks.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Backup {
    pub version: u32,
    pub created_at: chrono::DateTime<chrono::Local>,
    pub decks: Vec<Deck>,
}

/// Decks imported from a folder: (imported (name, card count), skipped names).
pub type FolderImportResult = (Vec<(String, usize)>, Vec<String>);

impl DeckStorage {
    /// Export all decks to a backup file.
    pub fn export_backup(&self, path: &Path) -> Result<usize> {
        let deck_infos = self.list_decks()?;
        let mut decks = Vec::new();

        for info in &deck_infos {
            if let Ok(Some(deck)) = self.load_deck(&info.id) {
                decks.push(deck);
            }
        }

        let backup = Backup {
            version: 1,
            created_at: chrono::Local::now(),
            decks,
        };

        let json = serde_json::to_string_pretty(&backup)?;
        fs::write(path, json)?;

        Ok(backup.decks.len())
    }

    /// Import decks from a backup file.
    /// Returns (imported_count, skipped_count).
    pub fn import_backup(&self, path: &Path) -> Result<(usize, usize)> {
        let json = fs::read_to_string(path)?;
        let backup: Backup = serde_json::from_str(&json)?;

        let existing_ids: std::collections::HashSet<String> =
            self.list_decks()?.into_iter().map(|d| d.id).collect();

        let mut imported = 0;
        let mut skipped = 0;

        for deck in backup.decks {
            if existing_ids.contains(&deck.id) {
                skipped += 1;
            } else {
                self.save_deck(&deck)?;
                imported += 1;
            }
        }

        Ok((imported, skipped))
    }

    /// Get default backup path.
    pub fn default_backup_path() -> PathBuf {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(format!("srl_backup_{}.json", timestamp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ReviewRating;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("srl_test_{}_{}", tag, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Drop an (invalid) .json placeholder into a fresh storage dir so
    /// install_bundled_decks() skips seeding the bundled deck — it would
    /// otherwise pollute deck-count and export assertions.
    fn suppress_bundled_deck(dir: &Path) {
        fs::write(dir.join(".placeholder.json"), "{}").unwrap();
    }

    // ── parse_csv_line ──────────────────────────────────────────────────

    #[test]
    fn csv_plain_fields() {
        assert_eq!(parse_csv_line("a,b"), vec!["a", "b"]);
        assert_eq!(parse_csv_line("a,b,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn csv_quoted_commas_and_escapes() {
        assert_eq!(parse_csv_line(r#""a,b",c"#), vec!["a,b", "c"]);
        assert_eq!(
            parse_csv_line(r#""say ""hi""",b"#),
            vec![r#"say "hi""#, "b"]
        );
        // A quoted field containing "front" is data, not a header
        assert_eq!(
            parse_csv_line(r#""the front",back"#),
            vec!["the front", "back"]
        );
    }

    // ── strip_html ──────────────────────────────────────────────────────

    #[test]
    fn br_tags_become_newlines() {
        assert_eq!(strip_html("Hello<br>World"), "Hello\nWorld");
        assert_eq!(strip_html("Hello<br/>World"), "Hello\nWorld");
        assert_eq!(strip_html("Hello<br />World"), "Hello\nWorld");
        assert_eq!(strip_html("<p>One</p><p>Two</p>"), "One\nTwo");
    }

    #[test]
    fn tags_stripped_entities_decoded() {
        assert_eq!(strip_html("<b>bold</b>"), "bold");
        assert_eq!(strip_html("a &amp; b &lt;c&gt;"), "a & b <c>");
        assert_eq!(strip_html("x&nbsp;y"), "x y");
    }

    // ── cloze handling (AnkiWeb 【高考╳Anki】 decks) ─────────────────────

    #[test]
    fn cloze_blank_and_answers() {
        let s = "{{c1::伽利略}}在比萨斜塔做了实验，提出{{c2::三条}}{{c1::运动定律}}";
        // c1 blanks out, c2 renders as plain text (first-template rendering)
        assert_eq!(cloze_blank(s), "＿＿＿在比萨斜塔做了实验，提出三条＿＿＿");
        assert_eq!(extract_cloze_answers(s), "伽利略；运动定律");
    }

    #[test]
    fn cloze_hint_shows_in_blank() {
        let s = "质量是{{c1::物体::对象的}}固有属性";
        assert_eq!(cloze_blank(s), "质量是［对象的］固有属性");
        assert_eq!(extract_cloze_answers(s), "物体");
    }

    #[test]
    fn multi_field_note_uses_first_nonempty_back() {
        // 成语-style template: field1 empty, field2 holds the meaning
        let flds = "安土重迁\u{1f}\u{1f}重迁，把搬迁看得很重。\u{1f}\u{1f}\u{1f}\u{1f}";
        let fields: Vec<&str> = flds.split('\u{1f}').collect();
        let back = fields[1..]
            .iter()
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        assert_eq!(back, "重迁，把搬迁看得很重。");
    }

    // ── filename_to_title_case ──────────────────────────────────────────

    #[test]
    fn filenames_to_title_case() {
        assert_eq!(filename_to_title_case("spanish_vocab"), "Spanish Vocab");
        assert_eq!(filename_to_title_case("gre-hard-words"), "Gre Hard Words");
        assert_eq!(filename_to_title_case("trailing_"), "Trailing");
    }

    // ── import_csv ──────────────────────────────────────────────────────

    #[test]
    fn csv_import_header_bom_quotes_and_tags() {
        let dir = temp_dir("csv");
        let path = dir.join("cards.csv");
        fs::write(
            &path,
            "\u{feff}front,back,tags\n\
             \"What is 2+2?\",\"Four\",math easy\n\
             What is 2+3?,Five,\n\
             \n\
             onlyfront,\n",
        )
        .unwrap();

        let storage = DeckStorage::new(dir.clone()).unwrap();
        let deck = storage.import_csv(&path, "T").unwrap();

        assert_eq!(deck.cards.len(), 2, "header/blank/empty-back rows skipped");
        assert_eq!(deck.cards[0].front, "What is 2+2?");
        assert_eq!(deck.cards[0].back, "Four");
        assert_eq!(deck.cards[0].tags, vec!["math", "easy"]);
        assert_eq!(deck.cards[1].back, "Five");
        assert!(deck.cards[1].tags.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn csv_import_first_row_with_front_word_is_data() {
        let dir = temp_dir("csv2");
        let path = dir.join("cards.csv");
        fs::write(&path, "the front side,answer\n").unwrap();

        let storage = DeckStorage::new(dir.clone()).unwrap();
        let deck = storage.import_csv(&path, "T").unwrap();
        assert_eq!(deck.cards.len(), 1, "not misdetected as a header row");

        let _ = fs::remove_dir_all(&dir);
    }

    // ── import_anki_text ────────────────────────────────────────────────

    #[test]
    fn anki_text_tab_semicolon_and_tags() {
        let dir = temp_dir("txt");
        let path = dir.join("vocab.txt");
        fs::write(
            &path,
            "hola\thello\tspanish basic\nbonjour;hello\n\n# comment\n",
        )
        .unwrap();

        let storage = DeckStorage::new(dir.clone()).unwrap();
        let deck = storage.import_anki_text(&path, "V").unwrap();

        assert_eq!(deck.cards.len(), 2);
        assert_eq!(deck.cards[0].front, "hola");
        assert_eq!(deck.cards[0].tags, vec!["spanish", "basic"]);
        assert_eq!(deck.cards[1].front, "bonjour");

        let _ = fs::remove_dir_all(&dir);
    }

    // ── apkg export → import roundtrip ─────────────────────────────────

    #[test]
    fn apkg_roundtrip_preserves_content_and_scheduling() {
        let src_dir = temp_dir("apkg_src");
        let dst_dir = temp_dir("apkg_dst");
        suppress_bundled_deck(&src_dir);

        let storage = DeckStorage::new(src_dir.clone()).unwrap();
        let mut deck = Deck::new("Roundtrip".into());
        deck.add_card("Front A".into(), "Back A".into());
        let studied = deck.add_card("Front B".into(), "Back B<br>line2".into());
        studied.interval = 10;
        studied.ease_factor = 2.6;
        studied.repetitions = 3;
        studied.lapses = 1;
        studied.total_reviews = 5;
        studied.due_date = Some(chrono::Local::now() + chrono::Duration::days(10));
        studied.last_reviewed = Some(chrono::Local::now() - chrono::Duration::days(1));
        studied.tags = vec!["geo".into()];
        storage.save_deck(&deck).unwrap();

        let apkg = src_dir.join("out.apkg");
        let exported = storage.export_apkg(&apkg, None).unwrap();
        assert_eq!(exported, 2);

        let dst = DeckStorage::new(dst_dir.clone()).unwrap();
        let decks = dst.import_apkg(&apkg).unwrap();
        assert_eq!(decks.len(), 1);
        assert_eq!(decks[0].name, "Roundtrip");
        assert_eq!(decks[0].cards.len(), 2);

        // Content, including restored line breaks from <br>
        assert_eq!(decks[0].cards[0].front, "Front A");
        assert_eq!(decks[0].cards[1].back, "Back B\nline2");
        assert_eq!(decks[0].cards[1].tags, vec!["geo"]);

        // Scheduling
        let b = &decks[0].cards[1];
        assert_eq!(b.interval, 10);
        assert!((b.ease_factor - 2.6).abs() < 1e-9);
        assert_eq!(b.repetitions, 3);
        assert_eq!(b.lapses, 1);
        // Due date should still be ~10 days out (not now + 10 days + elapsed)
        let days = (b.due_date.unwrap() - chrono::Local::now()).num_days();
        assert!(
            (9..=11).contains(&days),
            "due in ~10 days, got {} days",
            days
        );

        // A never-studied card keeps the default ease (Anki stores factor=0)
        assert!((decks[0].cards[0].ease_factor - 2.5).abs() < 1e-9);
        assert!(decks[0].cards[0].due_date.is_none());
        assert!(decks[0].cards[0].is_new());

        let _ = fs::remove_dir_all(&src_dir);
        let _ = fs::remove_dir_all(&dst_dir);
    }

    #[test]
    fn apkg_import_overdue_review_card_is_due_now() {
        let src_dir = temp_dir("apkg_overdue_src");
        let dst_dir = temp_dir("apkg_overdue_dst");
        suppress_bundled_deck(&src_dir);

        let storage = DeckStorage::new(src_dir.clone()).unwrap();
        let mut deck = Deck::new("Overdue".into());
        let c = deck.add_card("F".into(), "B".into());
        c.interval = 7;
        c.repetitions = 2;
        c.total_reviews = 3;
        c.due_date = Some(chrono::Local::now() - chrono::Duration::days(3)); // overdue
        storage.save_deck(&deck).unwrap();

        let apkg = src_dir.join("o.apkg");
        storage.export_apkg(&apkg, None).unwrap();

        let dst = DeckStorage::new(dst_dir.clone()).unwrap();
        let decks = dst.import_apkg(&apkg).unwrap();
        assert_eq!(decks[0].cards.len(), 1);
        assert!(
            decks[0].cards[0].is_due(),
            "overdue card must be due after import"
        );

        let _ = fs::remove_dir_all(&src_dir);
        let _ = fs::remove_dir_all(&dst_dir);
    }

    #[test]
    fn backup_roundtrip_and_duplicate_skip() {
        let src_dir = temp_dir("bk_src");
        let dst_dir = temp_dir("bk_dst");
        suppress_bundled_deck(&src_dir);
        suppress_bundled_deck(&dst_dir);

        let s1 = DeckStorage::new(src_dir.clone()).unwrap();
        let mut deck = Deck::new("BK".into());
        deck.add_card("f".into(), "b".into());
        s1.save_deck(&deck).unwrap();

        let backup = src_dir.join("bk.json");
        assert_eq!(s1.export_backup(&backup).unwrap(), 1);

        let s2 = DeckStorage::new(dst_dir.clone()).unwrap();
        assert_eq!(s2.import_backup(&backup).unwrap(), (1, 0));
        // Importing again: same deck id → skipped
        assert_eq!(s2.import_backup(&backup).unwrap(), (0, 1));
        assert_eq!(s2.list_decks().unwrap().len(), 1);

        let _ = fs::remove_dir_all(&src_dir);
        let _ = fs::remove_dir_all(&dst_dir);
    }

    #[test]
    fn saving_preserves_review_state() {
        let dir = temp_dir("save");
        let storage = DeckStorage::new(dir.clone()).unwrap();
        let mut deck = Deck::new("S".into());
        let id = deck.add_card("f".into(), "b".into()).id.clone();
        storage.save_deck(&deck).unwrap();

        let mut loaded = storage.load_deck(&deck.id).unwrap().unwrap();
        let card = loaded.cards.iter_mut().find(|c| c.id == id).unwrap();
        crate::sm2::Scheduler::new().review_card(card, ReviewRating::Good);
        storage.save_deck(&loaded).unwrap();

        let reloaded = storage.load_deck(&deck.id).unwrap().unwrap();
        assert_eq!(reloaded.cards[0].total_reviews, 1);
        assert_eq!(reloaded.cards[0].interval, 1);
        assert!(!reloaded.cards[0].is_new());

        let _ = fs::remove_dir_all(&dir);
    }
}
