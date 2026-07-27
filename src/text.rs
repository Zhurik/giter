//! Text shaping shared by the renderers. Names in this tool are long and share long
//! prefixes, so the layout dims what repeats instead of cutting off what does not.

/// Parts of a pod name the renderer dims differently: the workload prefix, the
/// ReplicaSet hash, and the suffix that actually tells one pod from another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodName<'a> {
    pub prefix: &'a str,
    pub replicaset: &'a str,
    pub suffix: &'a str,
}

/// Family a name belongs to: its first `-`-delimited prefix that at least one other name
/// shares. With `orchard-*` and `nimbus-*` side by side this picks `orchard-` and
/// `nimbus-`, and nothing for a name that stands alone.
pub fn family_prefix<'a>(name: &'a str, names: &[String]) -> &'a str {
    for (index, _) in name.char_indices().filter(|(_, c)| *c == '-') {
        let prefix = &name[..index + 1];

        // the name itself is in the list, so two matches mean one other name
        if names
            .iter()
            .filter(|other| other.starts_with(prefix))
            .count()
            >= 2
        {
            return prefix;
        }
    }

    ""
}

/// The part of a name that tells nothing, because every member of its family repeats it.
/// Dimmed rather than cut, so the telling part is always on screen.
pub fn shared_prefix<'a>(name: &'a str, names: &[String]) -> &'a str {
    let family = family_prefix(name, names);
    if family.is_empty() {
        return "";
    }

    let mut longest = family;

    for (index, _) in name.char_indices().filter(|(_, c)| *c == '-') {
        let candidate = &name[..index + 1];

        if candidate.len() > family.len()
            && names
                .iter()
                .filter(|other| other.starts_with(family))
                .all(|other| other.starts_with(candidate))
        {
            longest = candidate;
        }
    }

    longest
}

/// Splits `orchard-gateway-6b9f4c2d71-q8xzv` into the workload prefix, the
/// ReplicaSet hash and the unique suffix.
pub fn split_pod_name<'a>(pod: &'a str, namespace: &str) -> PodName<'a> {
    let (prefix, rest) = match pod.strip_prefix(namespace) {
        Some(rest) if rest.starts_with('-') => pod.split_at(namespace.len() + 1),
        _ => ("", pod),
    };

    match rest.rfind('-') {
        Some(cut) => PodName {
            prefix,
            replicaset: &rest[..cut + 1],
            suffix: &rest[cut + 1..],
        },
        None => PodName {
            prefix,
            replicaset: "",
            suffix: rest,
        },
    }
}

/// Compact age in the shape `kubectl` uses: `45s`, `12m`, `4h`, `6d`.
pub fn short_duration(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => format!("{}s", s),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        s => format!("{}d", s / 86_400),
    }
}

/// Fits `head` and `tail` into `width` characters by eating the head from the left,
/// because the head is the part every name repeats. The tail is only cut when it does not
/// fit on its own.
pub fn fit_head(head: &str, tail: &str, width: usize) -> (String, String) {
    let tail_len = tail.chars().count();
    let head_len = head.chars().count();

    if head_len + tail_len <= width {
        return (head.to_string(), tail.to_string());
    }

    if tail_len >= width {
        return (String::new(), ellipsize(tail, width));
    }

    match width - tail_len {
        0 => (String::new(), tail.to_string()),
        1 => ("…".to_string(), tail.to_string()),
        room => (
            format!(
                "…{}",
                head.chars().skip(head_len - room + 1).collect::<String>()
            ),
            tail.to_string(),
        ),
    }
}

/// Cuts `text` down to `width` characters, marking the cut with an ellipsis.
pub fn ellipsize(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }

    match width {
        0 => String::new(),
        1 => "…".to_string(),
        _ => text.chars().take(width - 1).chain(['…']).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speculoos::prelude::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn family_is_the_first_prefix_another_name_shares() {
        let all = names(&[
            "orchard-gateway",
            "orchard-ledger-api",
            "nimbus-index-component",
        ]);

        asserting!("a family groups a whole product, not one of its levels")
            .that(&family_prefix("orchard-ledger-api", &all))
            .is_equal_to("orchard-");
    }

    #[test]
    fn prefix_the_whole_family_repeats_is_dimmed_in_full() {
        let all = names(&["nimbus-cache-component", "nimbus-cache-warmer"]);

        assert_that!(shared_prefix("nimbus-cache-component", &all)).is_equal_to("nimbus-cache-");
    }

    #[test]
    fn prefix_stops_where_the_family_diverges() {
        let all = names(&[
            "nimbus-cache-component",
            "nimbus-cache-warmer",
            "nimbus-index-component",
        ]);

        asserting!("dimming nimbus-cache- would hide what tells the two apart")
            .that(&shared_prefix("nimbus-cache-component", &all))
            .is_equal_to("nimbus-");
    }

    #[test]
    fn name_outside_any_family_is_shown_whole() {
        let all = names(&["driftwood", "orchard-gateway", "orchard-ledger-api"]);

        assert_that!(shared_prefix("driftwood", &all)).is_equal_to("");
    }

    #[test]
    fn name_standing_alone_keeps_its_whole_label() {
        let all = names(&["pigeon-post", "orchard-gateway", "driftwood"]);

        assert_that!(family_prefix("pigeon-post", &all)).is_equal_to("");
    }

    #[test]
    fn name_without_a_separator_has_no_prefix() {
        let all = names(&["driftwood", "driftwood-proxy"]);

        assert_that!(family_prefix("driftwood", &all)).is_equal_to("");
    }

    #[test]
    fn pod_name_gives_up_the_namespace_prefix() {
        let parts = split_pod_name("orchard-gateway-6b9f4c2d71-q8xzv", "orchard-gateway");

        assert_that!(parts.prefix).is_equal_to("orchard-gateway-");
    }

    #[test]
    fn pod_name_keeps_the_replicaset_hash_apart() {
        let parts = split_pod_name("pigeon-post-58d3a0e4bc-mk2wr", "pigeon-post");

        assert_that!(parts.replicaset).is_equal_to("58d3a0e4bc-");
    }

    #[test]
    fn pod_name_ends_with_the_telling_suffix() {
        let suffix = "vd6np";
        let name = format!("tulip-mailer-7c1b9e5a04-{}", suffix);
        let parts = split_pod_name(&name, "tulip-mailer");

        assert_that!(parts.suffix).is_equal_to(suffix);
    }

    #[test]
    fn stateful_set_pod_has_no_replicaset_hash() {
        let parts = split_pod_name("driftwood-0", "driftwood");

        asserting!("an ordinal is not a ReplicaSet hash")
            .that(&parts.replicaset)
            .is_equal_to("");
    }

    #[test]
    fn pod_not_named_after_its_namespace_keeps_the_whole_name() {
        let parts = split_pod_name("mesh-ingress-abc12-xyz", "orchard-gateway");

        assert_that!(parts.prefix).is_equal_to("");
    }

    #[test]
    fn age_below_a_minute_is_counted_in_seconds() {
        assert_that!(short_duration(42)).is_equal_to("42s".to_string());
    }

    #[test]
    fn age_below_an_hour_is_counted_in_minutes() {
        assert_that!(short_duration(18 * 60 + 30)).is_equal_to("18m".to_string());
    }

    #[test]
    fn age_below_a_day_is_counted_in_hours() {
        assert_that!(short_duration(5 * 3_600)).is_equal_to("5h".to_string());
    }

    #[test]
    fn older_than_a_day_is_counted_in_days() {
        assert_that!(short_duration(6 * 86_400 + 7_000)).is_equal_to("6d".to_string());
    }

    #[test]
    fn clock_skew_does_not_produce_a_negative_age() {
        asserting!("a pod created in the future is shown as brand new")
            .that(&short_duration(-90))
            .is_equal_to("0s".to_string());
    }

    #[test]
    fn text_that_fits_is_left_alone() {
        let tag = "v1.4.7-8112430";

        assert_that!(ellipsize(tag, 20)).is_equal_to(tag.to_string());
    }

    #[test]
    fn text_that_does_not_fit_is_marked_as_cut() {
        assert_that!(ellipsize("CrashLoopBackOff", 9)).is_equal_to("CrashLoo…".to_string());
    }

    #[test]
    fn multibyte_text_is_cut_by_characters() {
        asserting!("cutting by bytes would split the ellipsis of the previous cut")
            .that(&ellipsize("не влезает", 4))
            .is_equal_to("не …".to_string());
    }

    #[test]
    fn no_room_at_all_yields_nothing() {
        assert_that!(ellipsize("forbidden", 0)).is_equal_to(String::new());
    }

    #[test]
    fn head_and_tail_that_fit_are_left_alone() {
        let (head, _) = fit_head("orchard-", "gateway", 20);

        assert_that!(head).is_equal_to("orchard-".to_string());
    }

    #[test]
    fn head_gives_up_room_before_the_tail_does() {
        let (head, _) = fit_head("orchard-ledger-", "worker", 16);

        asserting!("the repeated part is the one worth losing")
            .that(&head)
            .is_equal_to("…d-ledger-".to_string());
    }

    #[test]
    fn telling_tail_survives_a_narrow_column() {
        let suffix = "vd6np";
        let (_, tail) = fit_head("tulip-mailer-7c1b9e5a04-", suffix, 9);

        assert_that!(tail).is_equal_to(suffix.to_string());
    }

    #[test]
    fn tail_longer_than_the_column_is_cut_itself() {
        let (head, tail) = fit_head("nimbus-", "cache-component", 8);

        asserting!("nothing else can be given up at this point")
            .that(&(head, tail))
            .is_equal_to((String::new(), "cache-c…".to_string()));
    }
}
