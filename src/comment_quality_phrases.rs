const RATIONALE_MARKETING: &str = "state the fact instead of reaching for marketing language";
const RATIONALE_VAGUER_SYNONYM: &str = "say what actually happens instead of a vaguer synonym";
const RATIONALE_CUT_FILLER: &str = "cut the filler and state the fact directly";
const RATIONALE_SAY_BECAUSE: &str = "say \"because\" instead";

/// Marketing filler, hedge words, and invented-rationale phrases that add nothing a
/// reader couldn't already see, plus the task/fix/caller-referencing anti-pattern
/// (comments should describe the code, not the change that produced it — those belong
/// in the commit message instead, where they don't rot as the code moves on).
/// Matched case-insensitively as a substring, so keep entries lowercase.
pub(super) const BANNED_PHRASES: &[(&str, &str)] = &[
    ("seamlessly", RATIONALE_MARKETING),
    ("powerful", RATIONALE_MARKETING),
    ("robust", RATIONALE_MARKETING),
    ("enterprise-grade", RATIONALE_MARKETING),
    // The next two roots' inflected forms are listed as separate entries rather than
    // matched by a stem, per `contains_whole_phrase`'s word-boundary check below — a
    // real backtest finding: whole-word matching (needed so a short attribution
    // phrase doesn't fire inside an unrelated longer word) would otherwise also
    // reject a common third-person present-tense phrasing as "root plus more
    // letters," losing a legitimate hit.
    ("leverage", RATIONALE_VAGUER_SYNONYM),
    ("leverages", RATIONALE_VAGUER_SYNONYM),
    ("leveraging", RATIONALE_VAGUER_SYNONYM),
    ("leveraged", RATIONALE_VAGUER_SYNONYM),
    ("utilize", RATIONALE_VAGUER_SYNONYM),
    ("utilizes", RATIONALE_VAGUER_SYNONYM),
    ("utilizing", RATIONALE_VAGUER_SYNONYM),
    ("utilized", RATIONALE_VAGUER_SYNONYM),
    ("it's worth noting", RATIONALE_CUT_FILLER),
    ("as mentioned above", RATIONALE_CUT_FILLER),
    ("note that", RATIONALE_CUT_FILLER),
    ("needless to say", RATIONALE_CUT_FILLER),
    (
        "designed to improve",
        "only document what the code actually does, not the intent behind it",
    ),
    (
        "supports future",
        "only document what the code actually does, not speculative future use",
    ),
    (
        "used by",
        "this belongs in the commit message, not a comment that outlives its caller",
    ),
    (
        "added for the",
        "this belongs in the commit message, not a comment describing why it was added",
    ),
    (
        "this fix",
        "this belongs in the commit message, not a comment referencing the change",
    ),
    (
        "this pr",
        "this belongs in the commit message, not a comment referencing the change",
    ),
    (
        "handles the case from issue",
        "this belongs in the commit message, not a comment referencing an issue",
    ),
    // Bureaucratic wordy-filler phrases below, hand-picked from Vale's write-good
    // `TooWordy.yml` style pack (https://vale.sh/, vale-styles/write-good) — only the
    // unambiguous multi-word constructs that have no legitimate short form in a
    // technical comment. Deliberately NOT importing that pack's full list (or its
    // `Weasel.yml`): both are calibrated for narrative/bureaucratic prose and flag
    // ordinary technical vocabulary ("eliminate", "employ", "currently", "correctly")
    // that reads fine in a code comment — wholesale import would trade precision for
    // coverage in the wrong direction for this checker.
    (
        "in order to",
        "say \"to\" instead — \"in order to\" is always wordy filler",
    ),
    ("due to the fact that", RATIONALE_SAY_BECAUSE),
    ("because of the fact that", RATIONALE_SAY_BECAUSE),
    ("by virtue of the fact that", RATIONALE_SAY_BECAUSE),
    ("in spite of the fact that", "say \"although\" instead"),
    ("in the event that", "say \"if\" instead"),
    ("with regard to", "say \"about\" instead"),
    ("with regards to", "say \"about\" instead"),
    ("for the purpose of", "say \"to\" or \"for\" instead"),
    ("it is important to note that", RATIONALE_CUT_FILLER),
];
