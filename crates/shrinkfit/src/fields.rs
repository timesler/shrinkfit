//! Field scoring for the `condense` array strategy.
//!
//! When an array of similar records is too big, `condense` keeps every record
//! and drops the same low-value *fields* from all of them. Which fields are
//! low-value is decided by a [`FieldScorer`]: it sees statistics about one
//! field across all the records ([`ColumnInfo`]) and returns a score in
//! `0.0..=1.0` (higher = more worth keeping). The strategy keeps the best
//! fields that fit the budget.
//!
//! Scorers are small, pure and swappable:
//!
//! * [`ScorerKind`] names the built-in scorers (option `field_scorer`).
//! * Rust callers can pass their own with [`crate::shrink_with`].
//!
//! To add a built-in scorer: implement [`FieldScorer`], add a variant to
//! [`ScorerKind`], and register it in `ScorerKind::get` and `ScorerKind::NAMES`.

/// How many records had each kind of value for a field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KindCounts {
    pub null: usize,
    pub bool_: usize,
    pub number: usize,
    pub string: usize,
    pub array: usize,
    pub object: usize,
}

/// Statistics about one field (a key path relative to the array's records)
/// across all the records of an array.
#[derive(Clone, Debug)]
pub struct ColumnInfo<'a> {
    /// Key path from the record, e.g. `["author", "name"]`.
    pub path: &'a [String],
    /// Records in the array.
    pub total: usize,
    /// Records that have this field.
    pub present: usize,
    pub kinds: KindCounts,
    /// Present values that are empty: null, `""`, `[]` or `{}`.
    pub empty: usize,
    /// Distinct scalar values seen (capped), and how many scalars were counted.
    pub distinct: usize,
    pub scalars: usize,
    /// All present scalar values were identical.
    pub constant: bool,
    /// Every present value is a number or a string of digits (an opaque id
    /// rather than something readable like `OPS-12`).
    pub numeric_like: bool,
    /// Every present value is a string starting with an ISO date (`2024-05-01...`).
    pub timestamp_like: bool,
    /// How strongly the field's name or values mention what the caller asked
    /// for (`0.0..=1.0`; 0 when no arguments were given). See `lexical`.
    pub relevance: f64,
    /// The share of `relevance` that comes from the field's name (an explicit
    /// mention) and from its values (the query words occur in them).
    pub name_match: f64,
    pub value_match: f64,
    /// Fraction of the records whose value mentions a query word.
    pub match_fraction: f64,
    /// Most string values look machine-generated (UUIDs, hashes, random
    /// ids) rather than readable like `OPS-12`.
    pub opaque_like: bool,
    /// Average rendered size of the value, in characters.
    pub avg_chars: f64,
    /// Average position of the key among its siblings (0 = first).
    pub avg_position: f64,
}

impl ColumnInfo<'_> {
    /// The last key of the path.
    pub fn name(&self) -> &str {
        self.path.last().map(String::as_str).unwrap_or("")
    }

    /// Fraction of records that have the field.
    pub fn presence(&self) -> f64 {
        self.present as f64 / self.total.max(1) as f64
    }
}

/// Scores how worth keeping a field is. Implementations must be pure.
pub trait FieldScorer: Send + Sync {
    /// Name used in options and reports.
    fn name(&self) -> &'static str;
    /// Score in `0.0..=1.0`; higher means keep in preference to lower.
    fn score(&self, column: &ColumnInfo<'_>) -> f64;

    /// Must this field's values come out exactly as they are (identifiers,
    /// URLs used as handles, cursors)? Such values are never truncated; if they
    /// do not fit, the field is dropped rather than corrupted. Default: no.
    fn exact(&self, _column: &ColumnInfo<'_>) -> bool {
        false
    }

    /// Optional role of the field (e.g. `"identity"`). Fields sharing a role
    /// are partly redundant for a reader, so after the best one the others
    /// are discounted. `None` (the default) means no role.
    fn role(&self, _column: &ColumnInfo<'_>) -> Option<String> {
        None
    }
}

/// Names of the built-in scorers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScorerKind {
    /// Name cues + value statistics (the default).
    Heuristic,
    /// Prefers small fields: value per character, no name knowledge.
    Compact,
    /// Positional baseline: earlier keys win.
    KeyOrder,
}

static HEURISTIC: HeuristicScorer = HeuristicScorer;
static COMPACT: CompactScorer = CompactScorer;
static KEY_ORDER: KeyOrderScorer = KeyOrderScorer;

impl ScorerKind {
    pub const NAMES: [&'static str; 3] = ["heuristic", "compact", "key_order"];

    pub fn get(self) -> &'static dyn FieldScorer {
        match self {
            ScorerKind::Heuristic => &HEURISTIC,
            ScorerKind::Compact => &COMPACT,
            ScorerKind::KeyOrder => &KEY_ORDER,
        }
    }

    pub fn name(self) -> &'static str {
        self.get().name()
    }

    pub fn from_name(s: &str) -> Option<ScorerKind> {
        match s.to_ascii_lowercase().replace('-', "_").as_str() {
            "heuristic" => Some(ScorerKind::Heuristic),
            "compact" => Some(ScorerKind::Compact),
            "key_order" | "keyorder" => Some(ScorerKind::KeyOrder),
            _ => None,
        }
    }
}

/// Baseline: keep earlier keys. Useful as a floor when comparing scorers.
pub struct KeyOrderScorer;

impl FieldScorer for KeyOrderScorer {
    fn name(&self) -> &'static str {
        "key_order"
    }

    fn score(&self, c: &ColumnInfo<'_>) -> f64 {
        1.0 / (1.0 + c.avg_position)
    }
}

/// Baseline: prefer small fields, ignore names.
pub struct CompactScorer;

impl FieldScorer for CompactScorer {
    fn name(&self) -> &'static str {
        "compact"
    }

    fn score(&self, c: &ColumnInfo<'_>) -> f64 {
        let mut s = 1.0 / (1.0 + c.avg_chars / 40.0);
        if c.empty == c.present || c.constant {
            s *= 0.1;
        }
        s
    }
}

/// The default scorer: name cues combined with value statistics.
///
/// 1. A **name tier** gives a prior: identifiers, titles and statuses high;
///    boilerplate such as links, avatars and expansion hints low.
/// 2. **Value statistics** adjust it: fields that are always empty or the
///    same in every record carry no per-record information; short,
///    all-distinct fields look like identifiers; low-variety fields look
///    like categories; very long fields are costly.
pub struct HeuristicScorer;

/// Mongo extended JSON wraps values (`{"$oid": ..}`, `{"$date": ..}`): score the
/// field that holds the wrapper, not the wrapper key.
fn eff_path(path: &[String]) -> &[String] {
    match path.split_last() {
        Some((last, rest)) if last.starts_with('$') && !rest.is_empty() => rest,
        _ => path,
    }
}

/// How much a field mentioned by the call's arguments gains, at full relevance.
const RELEVANCE_BOOST: f64 = 0.4;
/// A value match never lifts a field above this (identity and titles stay above
/// anything whose values merely contain a query word), unless its score is
/// already higher, e.g. because the caller named the field.
const BOOST_CAP: f64 = 0.8;

/// Split a key into lowercase words: `html_url`, `htmlUrl`, `HTML-URL`.
fn words(key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in key.chars() {
        if ch.is_alphanumeric() {
            if ch.is_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.extend(ch.to_lowercase());
            prev_lower = ch.is_lowercase() || ch.is_numeric();
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

const IDENTITY: &[&str] = &[
    "ari",
    "arn",
    "urn",
    "gid",
    "sha",
    "oid",
    "id",
    "key",
    "uuid",
    "guid",
    "slug",
    "number",
    "ref",
    "sku",
    "handle",
    "pk",
    "identifier",
];
/// What a record is called: the best single thing to show about it.
/// How to reach a person or place.
const CONTACT: &[&str] = &["email", "phone", "mobile", "telephone", "address"];
const TITLE: &[&str] = &[
    "title", "summary", "subject", "headline", "heading", "caption",
];
const LABEL: &[&str] = &[
    "firstname",
    "lastname",
    "givenname",
    "familyname",
    "surname",
    "nickname",
    "name",
    "label",
    "displayname",
    "fullname",
    "username",
    "login",
    "text",
];
/// Envelope keys that only wrap a record's real fields (`fields/summary`).
const WRAPPERS: &[&str] = &[
    "content",
    "resource",
    "object",
    "source",
    "embedded",
    "doc",
    "document",
    "properties",
    "fields",
    "attributes",
    "data",
    "node",
    "item",
    "entity",
    "record",
];
const STATUS: &[&str] = &[
    "completed",
    "done",
    "active",
    "archived",
    "enabled",
    "verified",
    "paid",
    "phase",
    "health",
    "condition",
    "conditions",
    "availability",
    "verdict",
    "status",
    "state",
    "type",
    "kind",
    "category",
    "resolution",
    "priority",
    "severity",
    "level",
    "error",
    "errors",
    "code",
    "result",
    "success",
    "ok",
    "reason",
    "role",
    "stage",
    "outcome",
];
const CONTENT: &[&str] = &[
    "description",
    "body",
    "content",
    "abstract",
    "snippet",
    "excerpt",
    "comment",
    "note",
    "details",
    "message",
    "overview",
    "bio",
    "value",
];
const PEOPLE: &[&str] = &[
    "author",
    "creator",
    "assignee",
    "reporter",
    "owner",
    "user",
    "by",
    "sender",
    "from",
    "lastmodifiedby",
    "createdby",
];
const TIME: &[&str] = &[
    "ts",
    "created",
    "updated",
    "modified",
    "date",
    "time",
    "timestamp",
    "at",
    "due",
    "start",
    "end",
    "published",
    "when",
    "deadline",
    "resolved",
];
const METRIC: &[&str] = &[
    "points",
    "comments",
    "replies",
    "downloads",
    "followers",
    "forks",
    "watchers",
    "upvotes",
    "count",
    "total",
    "score",
    "rank",
    "size",
    "length",
    "stars",
    "votes",
    "likes",
    "views",
    "price",
    "amount",
    "quantity",
    "rating",
    "relevance",
    "distance",
];
const LINK: &[&str] = &["url", "uri", "link", "href", "permalink", "webui", "web"];
/// Boilerplate: rarely what a reader needs.
const NOISE: &[&str] = &[
    "etag",
    "hash",
    "checksum",
    "schema",
    "revision",
    "rev",
    "expand",
    "expandable",
    "self",
    "avatar",
    "avatars",
    "icon",
    "thumbnail",
    "image",
    "images",
    "logo",
    "picture",
    "color",
    "colour",
    "links",
    "typename",
    "__typename",
    "embedded",
    "context",
    "tinyui",
    "base",
    "profilepicture",
    "avatarurls",
    "iconurl",
];
/// Containers of mostly secondary detail.
const SECONDARY: &[&str] = &[
    "metadata",
    "meta",
    "extensions",
    "history",
    "restrictions",
    "operations",
    "permissions",
    "visibility",
    "properties",
    "settings",
    "config",
    "flags",
    "attributes",
    "customfield",
    "version",
    "versions",
];

fn any(ws: &[String], list: &[&str]) -> bool {
    ws.iter().any(|w| list.contains(&w.as_str()))
}

/// Tier of a set of words, checking the list order below.
fn tier_of(ws: &[String]) -> Option<f64> {
    // Identity first: tiny, and what a reader needs to refer to a record.
    if any(ws, IDENTITY) {
        Some(0.98)
    } else if any(ws, TITLE) {
        Some(0.95)
    } else if any(ws, LABEL) {
        Some(0.9)
    } else if any(ws, STATUS) {
        Some(0.85)
    } else if any(ws, CONTACT) {
        Some(0.75)
    } else if any(ws, CONTENT) {
        Some(0.7)
    } else if any(ws, PEOPLE) {
        Some(0.65)
    } else if any(ws, METRIC) || any(ws, LINK) {
        Some(0.6)
    } else if any(ws, TIME) {
        Some(0.55)
    } else {
        None
    }
}

/// Tier of a field name. The last word is the head noun and decides first
/// (`overall_state_modified` is a time, not a state); run-together names
/// (`firstname`, `jobtitle`, `lastmodifieddate`) are matched by their ending.
fn tier(ws: &[String], joined: &str) -> Option<f64> {
    if let Some(last) = ws.last() {
        if let Some(t) = tier_of(std::slice::from_ref(last)) {
            // `branchName`, `projectName`: a name of something else, not of the
            // record. Person-name qualifiers (`displayName`) keep the full tier.
            const OWN_NAME: &[&str] = &[
                "display", "full", "first", "last", "given", "family", "user", "nick", "short",
                "sur", "login",
            ];
            if last == "login" && ws.len() > 1 {
                return Some(0.55); // lastLogin, loginAt: a time
            }
            let qualified = LABEL.contains(&last.as_str())
                && ws.len() > 1
                && !ws[..ws.len() - 1]
                    .iter()
                    .all(|w| OWN_NAME.contains(&w.as_str()));
            return Some(if qualified { t.min(0.78) } else { t });
        }
    }
    if let Some(t) = tier_of(ws) {
        return Some(t);
    }
    if ws.len() == 1 && joined.len() > 5 {
        let lists: [&[&str]; 9] = [
            TITLE, LABEL, STATUS, CONTACT, CONTENT, PEOPLE, METRIC, LINK, TIME,
        ];
        let best = lists
            .iter()
            .flat_map(|l| l.iter())
            .filter(|w| w.len() >= 4 && joined.ends_with(**w))
            .max_by_key(|w| w.len());
        if let Some(w) = best {
            return tier_of(&[w.to_string()]);
        }
    }
    None
}

impl HeuristicScorer {
    /// Prior from the field's name (and its ancestors').
    /// Returns the prior and whether the name (or its values) identified the kind of field.
    fn name_prior(&self, c: &ColumnInfo<'_>) -> (f64, bool) {
        let path = eff_path(c.path);
        let own = words(path.last().map(String::as_str).unwrap_or(""));
        let joined = own.concat();
        let ancestors: Vec<String> = c
            .path
            .iter()
            .take(c.path.len().saturating_sub(1))
            .flat_map(|k| words(k))
            .collect();

        // Anything inside a boilerplate container (`_links`, `avatarUrls`) is noise.
        if !ancestors.is_empty() && any(&ancestors, NOISE) {
            return (0.08, true);
        }
        // `_links`, `_highlightResult`, `__typename`: API plumbing, not data.
        // (`_id` and `_source` are Mongo/Elasticsearch data.)
        let plumbing = |k: &String| {
            k.is_empty()
                || (k.starts_with('_') && !matches!(k.as_str(), "_id" | "_source" | "_embedded"))
        };
        if path.iter().any(plumbing) {
            return (0.08, true);
        }
        if any(&own, NOISE) || NOISE.contains(&joined.as_str()) {
            // `html_url` / `webui` are real links even though they contain "url".
            if !any(&own, LINK)
                || any(
                    &own,
                    &[
                        "avatar",
                        "icon",
                        "thumbnail",
                        "image",
                        "logo",
                        "picture",
                        "self",
                    ],
                )
            {
                return (0.1, true);
            }
        }
        if any(&own, SECONDARY) {
            return (0.25, true);
        }
        // Nesting inside a real sub-object (not an envelope like `fields`).
        let nested = path
            .iter()
            .take(path.len().saturating_sub(1))
            .any(|k| !words(k).iter().all(|w| WRAPPERS.contains(&w.as_str())));
        let tiered = tier(&own, &joined);
        let mut p = tiered.unwrap_or(0.5);
        // Unknown name but ISO dates inside: it is a timestamp.
        if tiered.is_none() && c.timestamp_like {
            p = 0.55;
        }
        // A link that is different in every record is a retrieval handle for
        // it (a page URL, an embed link): worth keeping like an identifier.
        if any(&own, LINK)
            && c.kinds.string == c.present
            && c.scalars >= 3
            && c.distinct * 10 >= c.scalars * 9
        {
            p = p.max(0.8);
        }
        // Among identifiers, readable ones (`OPS-12`) beat opaque numbers.
        if c.numeric_like && any(&own, IDENTITY) {
            p -= 0.04;
        }
        if any(&own, IDENTITY) {
            // `node_id`, `place_id`, `userId`: references to something else.
            if own.len() > 1 && own.last().map(String::as_str) == Some("id") {
                // ...and opaque ones (`client_msg_id`) are plumbing.
                p = p.min(if c.opaque_like { 0.3 } else { 0.72 });
            }
            // Machine-generated ids (`tblP1F8hoChkS253A`, UUIDs) are opaque;
            // names and readable keys read better.
            if c.opaque_like || c.avg_chars > 16.0 {
                p -= 0.1;
            }
        }
        // Fields nested under sub-objects keep their meaning but are less
        // central; an id inside one is a foreign key, and the sub-object's
        // name is what reads.
        if nested {
            p *= if any(&own, IDENTITY) { 0.6 } else { 0.88 };
        }
        (p, tiered.is_some() || c.timestamp_like)
    }
}

impl FieldScorer for HeuristicScorer {
    fn name(&self) -> &'static str {
        "heuristic"
    }

    fn score(&self, c: &ColumnInfo<'_>) -> f64 {
        let (mut s, known) = self.name_prior(c);
        let status_like = any(&words(c.name()), STATUS);

        // Always empty: nothing to read.
        if c.present > 0 && c.empty == c.present {
            return (s * 0.03).min(0.05);
        }
        // Mostly empty: little to read in most records.
        let empty_fraction = c.empty as f64 / c.present.max(1) as f64;
        if empty_fraction > 0.3 {
            s *= 1.0 - 0.7 * empty_fraction;
        }
        // Identical everywhere: no per-record information.
        if c.constant && c.total >= 3 && c.scalars == c.present {
            s *= 0.2;
        }
        // Fields only some records have say less about the array as a whole;
        // status-like ones (errors, reasons) are kept for the exceptions.
        let presence = c.presence();
        s *= if status_like {
            0.85 + 0.15 * presence
        } else {
            0.55 + 0.45 * presence
        };
        // Unknown names: let the data decide.
        let unknown = !known;
        // (Only strings: unique numbers are as likely measurements as ids.)
        // (Numbers written as text are measurements, not names or ids.)
        if unknown
            && c.scalars >= 3
            && c.scalars == c.present
            && c.kinds.string == c.present
            && !c.numeric_like
        {
            let ratio = c.distinct as f64 / c.scalars as f64;
            if c.opaque_like {
                s = 0.3; // machine-generated (cursors, tokens, hashes): plumbing
            } else if ratio >= 0.9 && c.avg_chars <= 40.0 {
                s = 0.7; // looks like an identifier
            } else if ratio <= 0.2 && c.avg_chars <= 24.0 && !c.constant {
                s = 0.65; // looks like a category
            }
        }
        // Mentioned by the tool call's arguments: what the caller asked about.
        // (Identical in every record means it is the filter itself, an echo.)
        // The boost scales with the field's own prior, so a match inside an
        // avatar URL does not turn plumbing into content.
        if c.relevance > 0.0 && !c.constant {
            // A category field whose values match the query is mostly the filter
            // itself coming back; it says little unless only some records match.
            let category = c.kinds.string == c.present
                && c.distinct * 5 <= c.scalars.max(1)
                && c.avg_chars <= 24.0;
            let damp = if category {
                2.0 * c.match_fraction * (1.0 - c.match_fraction)
            } else {
                1.0
            };
            // An explicit mention (a `SELECT` list, `fields=`) is a request: it
            // may outrank anything. A value match is only evidence, so it is
            // capped below identifiers and titles.
            let scale = (2.0 * s).clamp(0.25, 1.0);
            let named = (s + RELEVANCE_BOOST * c.name_match * scale).min(1.0);
            let value_part = RELEVANCE_BOOST * (c.value_match * damp) * scale;
            s = (named + value_part).min(named.max(BOOST_CAP));
        }
        // Costly values: still useful (they can be shortened) but later.
        if c.avg_chars > 2000.0 {
            s *= 0.7;
        } else if c.avg_chars > 200.0 {
            s *= 0.85;
        }
        // Containers of unknown content are a gamble.
        if c.kinds.array + c.kinds.object > c.present / 2 && s < 0.8 {
            s *= 0.85;
        }
        s.clamp(0.0, 1.0)
    }

    fn exact(&self, c: &ColumnInfo<'_>) -> bool {
        if c.present == 0 || c.avg_chars > 300.0 || c.kinds.string + c.kinds.number != c.present {
            return false;
        }
        let path = eff_path(c.path);
        let own = words(path.last().map(String::as_str).unwrap_or(""));
        let unique = c.scalars >= 3 && c.distinct * 10 >= c.scalars * 9;
        // Identifiers, machine-generated ids and per-record links are worthless
        // once shortened.
        (any(&own, IDENTITY) && !c.constant) || (unique && (c.opaque_like || any(&own, LINK)))
    }

    fn role(&self, c: &ColumnInfo<'_>) -> Option<String> {
        let path = eff_path(c.path);
        let parents = &path[..path.len().saturating_sub(1)];
        let real: Vec<&String> = parents
            .iter()
            .filter(|k| !words(k).iter().all(|w| WRAPPERS.contains(&w.as_str())))
            .collect();
        if !real.is_empty() {
            // Several fields about the same sub-object (`author/login`,
            // `author/name`, `author/email`) mostly say the same thing.
            return Some(format!(
                "group:{}",
                real.iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join("/")
            ));
        }
        // Top-level identifiers (`id`, `key`, `number`) tell a reader the
        // same thing: which record this is. Likewise titles, times, links.
        let own = words(path.last().map(String::as_str).unwrap_or(""));
        if any(&own, IDENTITY) {
            Some("identity".into())
        } else if any(&own, TITLE) {
            Some("title".into())
        } else if any(&own, TIME) {
            Some("time".into())
        } else if any(&own, LINK) {
            Some("link".into())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col<'a>(path: &'a [String]) -> ColumnInfo<'a> {
        ColumnInfo {
            path,
            total: 10,
            present: 10,
            kinds: KindCounts {
                string: 10,
                ..Default::default()
            },
            empty: 0,
            distinct: 10,
            scalars: 10,
            constant: false,
            numeric_like: false,
            timestamp_like: false,
            opaque_like: false,
            relevance: 0.0,
            name_match: 0.0,
            value_match: 0.0,
            match_fraction: 0.0,
            avg_chars: 12.0,
            avg_position: 3.0,
        }
    }

    fn score(path: &[&str], tweak: impl Fn(&mut ColumnInfo<'_>)) -> f64 {
        let p: Vec<String> = path.iter().map(|s| s.to_string()).collect();
        let mut c = col(&p);
        tweak(&mut c);
        HeuristicScorer.score(&c)
    }

    #[test]
    fn words_split_all_cases() {
        assert_eq!(words("html_url"), ["html", "url"]);
        assert_eq!(words("htmlUrl"), ["html", "url"]);
        assert_eq!(words("avatarUrls"), ["avatar", "urls"]);
        assert_eq!(words("ID"), ["id"]);
    }

    #[test]
    fn names_rank_as_expected() {
        let none = |_: &mut ColumnInfo<'_>| {};
        let id = score(&["id"], none);
        let title = score(&["title"], none);
        let status = score(&["status"], none);
        let created = score(&["created"], none);
        let avatar = score(&["avatar_url"], none);
        let links = score(&["_links", "self"], none);
        assert!(
            id > title && title > status && status > created,
            "{id} {title} {status} {created}"
        );
        assert!(created > avatar && created > links);
        assert!(avatar < 0.2 && links < 0.2);
        // A real link stays moderately valuable.
        assert!(score(&["html_url"], none) > 0.5);
    }

    #[test]
    fn statistics_adjust_the_prior() {
        let none = |_: &mut ColumnInfo<'_>| {};
        let empty = score(&["title"], |c| c.empty = c.present);
        assert!(empty < 0.06);
        let constant = score(&["type"], |c| {
            c.constant = true;
            c.distinct = 1;
        });
        assert!(constant < score(&["type"], none));
        // Unknown name, all-distinct short values -> identifier-like.
        assert!(score(&["zzz"], none) >= 0.7);
        // Unknown name, low variety -> category-like.
        assert!(score(&["zzz"], |c| c.distinct = 2) >= 0.6);
    }

    #[test]
    fn spot_check_regressions() {
        let none = |_: &mut ColumnInfo<'_>| {};
        // Run-together names are recognised by their ending (HubSpot).
        assert!(score(&["properties", "firstname"], none) > 0.7);
        assert!(score(&["properties", "email"], none) > 0.6);
        // The last word decides: a state's modification time is a time (Datadog).
        assert!(score(&["overall_state_modified"], none) < score(&["overall_state"], none));
        // Kubernetes phase is a status.
        assert!(score(&["status", "phase"], none) > 0.7);
        // `branchName` names something else, below a plain `name`.
        assert!(score(&["branchName"], none) < score(&["name"], none));
        // Timestamps with unknown names are not identifiers (Salesforce).
        let ts = score(&["SystemModstamp"], |c| c.timestamp_like = true);
        assert!(ts < 0.6, "{ts}");
        // Decimal numbers as text are measurements, not ids (OHLC prices).
        let price = score(&["close"], |c| c.numeric_like = true);
        assert!(price <= 0.5, "{price}");
        // Opaque references are plumbing.
        assert!(score(&["client_msg_id"], |c| c.opaque_like = true) < 0.35);
        // API plumbing keys are noise at any depth.
        assert!(score(&["_highlightResult", "title", "value"], none) < 0.1);
        // Mostly-empty fields lose to full ones.
        assert!(score(&["priority"], |c| c.empty = 7) < score(&["priority"], none) * 0.6);
    }

    #[test]
    fn argument_boost_is_bounded_and_ignores_echoes() {
        let none = |_: &mut ColumnInfo<'_>| {};
        let plain = score(&["zzz"], none);
        // A value match helps, but never beyond the cap, so identifiers and
        // titles stay on top...
        let valued = score(&["zzz"], |c| {
            c.relevance = 1.0;
            c.value_match = 1.0;
            c.match_fraction = 0.5;
        });
        assert!(valued > plain);
        assert!(valued <= plain.max(BOOST_CAP) + 1e-9, "{valued}");
        assert!(score(&["id"], none) > valued);
        // ...while an explicit mention (`SELECT zzz`) is a request and may
        // outrank anything.
        let named = score(&["zzz"], |c| {
            c.relevance = 1.0;
            c.name_match = 1.0;
        });
        assert!(named > valued, "{named} vs {valued}");
        // A category field whose values match the query is mostly the filter
        // echoing back: no boost when every record matches, some when few do.
        let category = |matched: f64, rel: f64| {
            move |c: &mut ColumnInfo<'_>| {
                c.kinds = KindCounts {
                    string: 10,
                    ..Default::default()
                };
                c.distinct = 2;
                c.scalars = 10;
                c.avg_chars = 8.0;
                c.relevance = rel;
                c.value_match = rel;
                c.match_fraction = matched;
            }
        };
        let plain = score(&["kind2"], category(0.0, 0.0));
        let echo = score(&["kind2"], category(1.0, 1.0));
        let partial = score(&["kind2"], category(0.5, 1.0));
        assert!((echo - plain).abs() < 1e-9, "{echo} vs {plain}");
        assert!(partial > echo, "{partial} vs {echo}");
    }

    #[test]
    fn scorers_resolve_by_name() {
        for n in ScorerKind::NAMES {
            assert_eq!(ScorerKind::from_name(n).unwrap().name(), n);
        }
        assert!(ScorerKind::from_name("nope").is_none());
    }
}
