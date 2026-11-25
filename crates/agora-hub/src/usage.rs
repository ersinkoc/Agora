//! Usage and cost (docs/design/usage-and-cost.md, ADR-0021): token usage times catalog prices. Pure functions and
//! plain data. Money is kept in micro-USD (1 USD = 1_000_000) as integers so sums never drift; prices are micro-USD
//! per million tokens, which makes `tokens * price / 1_000_000` exact in micro-USD before rounding.
//!
//! Rules from the design doc: an agent-reported cost always wins over the estimate; a model without a price gives
//! no estimate (never a guess); an override beats the catalog.

use std::collections::BTreeMap;

use serde_json::Value;

/// Price of one million tokens, in micro-USD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Price {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(other.cache_write_tokens);
    }
}

/// Where a cost figure came from; the UI shows the difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cost {
    /// The agent or adapter said what it cost.
    Reported(u64),
    /// Computed from the catalog (or an override).
    Estimated(u64),
}

impl Cost {
    pub fn micro_usd(self) -> u64 {
        match self {
            Self::Reported(v) | Self::Estimated(v) => v,
        }
    }
}

/// Providers whose own price is taken for a bare model id several providers list.
const FIRST_PARTY: [&str; 6] = [
    "anthropic",
    "openai",
    "google",
    "mistral",
    "deepseek",
    "xai",
];

#[derive(Debug, Default, Clone)]
pub struct Catalog {
    prices: BTreeMap<String, Price>,
    overrides: BTreeMap<String, Price>,
}

/// USD per million tokens as a JSON number → micro-USD per million tokens. Negative or non-finite values are not prices.
fn micro(v: &Value) -> Option<u64> {
    let text = v.as_number()?.to_string();
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
        None => (text.as_str(), 0),
    };
    if mantissa.starts_with('-') {
        return None;
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let shift = exponent
        .checked_add(6)?
        .checked_sub(i32::try_from(fraction.len()).ok()?)?;
    if shift >= 0 {
        let zeros = usize::try_from(shift).ok()?;
        if digits.len().saturating_add(zeros) > 20 {
            return None;
        }
        return format!("{digits}{}", "0".repeat(zeros)).parse().ok();
    }
    let cut = i64::try_from(digits.len())
        .ok()?
        .checked_add(i64::from(shift))?;
    if cut < 0 {
        return Some(0);
    }
    let cut = usize::try_from(cut).ok()?;
    let mut rounded = if cut == 0 {
        0
    } else {
        digits.get(..cut)?.parse::<u64>().ok()?
    };
    if digits
        .as_bytes()
        .get(cut)
        .is_some_and(|digit| *digit >= b'5')
    {
        rounded = rounded.checked_add(1)?;
    }
    Some(rounded)
}

impl Catalog {
    /// Reads the models.dev `api.json` shape: `{ provider: { models: { model: { cost: { input, output,
    /// cache_read, cache_write } } } } }`, keyed here as `provider/model`. Models without a usable `cost` are
    /// skipped; a missing cache price counts as 0 (no separate charge known).
    pub fn from_models_dev(json: &str) -> Result<Self, serde_json::Error> {
        let root: Value = serde_json::from_str(json)?;
        let mut prices = BTreeMap::new();
        if let Some(providers) = root.as_object() {
            for (pid, p) in providers {
                let Some(models) = p.get("models").and_then(Value::as_object) else {
                    continue;
                };
                for (mid, m) in models {
                    let Some(cost) = m.get("cost") else { continue };
                    let (Some(input), Some(output)) = (
                        cost.get("input").and_then(micro),
                        cost.get("output").and_then(micro),
                    ) else {
                        continue;
                    };
                    let cache_read = match cost.get("cache_read") {
                        Some(value) => {
                            let Some(value) = micro(value) else { continue };
                            value
                        }
                        None => 0,
                    };
                    let cache_write = match cost.get("cache_write") {
                        Some(value) => {
                            let Some(value) = micro(value) else { continue };
                            value
                        }
                        None => 0,
                    };
                    prices.insert(
                        format!("{pid}/{mid}"),
                        Price {
                            input,
                            output,
                            cache_read,
                            cache_write,
                        },
                    );
                }
            }
        }
        Ok(Self {
            prices,
            overrides: BTreeMap::new(),
        })
    }

    /// A negotiated price or a private model; beats the catalog.
    pub fn set_override(&mut self, model: &str, price: Price) {
        self.overrides.insert(model.to_string(), price);
    }

    /// The price of `model`: an override, then the catalog by the id as given (`provider/model`), then a bare model id
    /// (what agents usually report) by its first-party provider, or by the only provider that lists it. A bare id that
    /// several providers list at their own prices has no price rather than a guess.
    pub fn price(&self, model: &str) -> Option<Price> {
        if model.is_empty() {
            return None;
        }
        if let Some(p) = self.overrides.get(model).or_else(|| self.prices.get(model)) {
            return Some(*p);
        }
        if model.contains('/') {
            return None;
        }
        let mut by_provider = BTreeMap::new();
        for (key, price) in self.prices.iter().chain(&self.overrides) {
            let Some((provider, candidate)) = key.split_once('/') else {
                continue;
            };
            if candidate == model {
                by_provider.insert(provider, *self.overrides.get(key).unwrap_or(price));
            }
        }
        let found: Vec<(&str, Price)> = by_provider.into_iter().collect();
        if let [(_, p)] = found.as_slice() {
            return Some(*p);
        }
        if let Some((_, first)) = found.first() {
            if found.iter().all(|(_, price)| price == first) {
                return Some(*first);
            }
        }
        FIRST_PARTY
            .iter()
            .find_map(|f| found.iter().find(|(prov, _)| prov == f).map(|(_, p)| *p))
    }

    /// Reads the hub owner's override file: `{ "<model>": { "input": USD, "output": USD, "cache_read": USD,
    /// "cache_write": USD } }`, USD per million tokens. Entries without an input and output price are skipped; the
    /// count of skipped entries is returned.
    pub fn load_overrides(&mut self, json: &str) -> Result<usize, serde_json::Error> {
        let root: Value = serde_json::from_str(json)?;
        let mut skipped = 0;
        for (model, p) in root.as_object().into_iter().flatten() {
            match (
                p.get("input").and_then(micro),
                p.get("output").and_then(micro),
            ) {
                (Some(input), Some(output)) => self.set_override(
                    model,
                    Price {
                        input,
                        output,
                        cache_read: p.get("cache_read").and_then(micro).unwrap_or(0),
                        cache_write: p.get("cache_write").and_then(micro).unwrap_or(0),
                    },
                ),
                _ => skipped += 1,
            }
        }
        Ok(skipped)
    }

    /// How many models have a catalog price (overrides not counted).
    pub fn len(&self) -> usize {
        self.prices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }

    /// The table that ships with the release (`scripts/update-prices.sh`), parsed once.
    pub fn bundled() -> std::sync::Arc<Catalog> {
        static BUNDLED: std::sync::OnceLock<std::sync::Arc<Catalog>> = std::sync::OnceLock::new();
        std::sync::Arc::clone(BUNDLED.get_or_init(|| {
            std::sync::Arc::new(
                Catalog::from_models_dev(include_str!("../data/prices.json")).unwrap_or_default(),
            )
        }))
    }

    /// Cost of `usage` on `model`: the reported value if there is one, else an estimate, else `None`.
    pub fn cost(
        &self,
        model: &str,
        usage: &Usage,
        reported_micro_usd: Option<u64>,
    ) -> Option<Cost> {
        if let Some(r) = reported_micro_usd {
            return Some(Cost::Reported(r));
        }
        let p = self.price(model)?;
        let parts = [
            (usage.input_tokens, p.input),
            (usage.output_tokens, p.output),
            (usage.cache_read_tokens, p.cache_read),
            (usage.cache_write_tokens, p.cache_write),
        ];
        // u128 keeps real usage exact and the division rounds once; synthetic boundary inputs saturate.
        let total: u128 = parts
            .iter()
            .map(|(t, price)| u128::from(*t) * u128::from(*price))
            .fold(0, u128::saturating_add);
        let micro = total.saturating_add(500_000) / 1_000_000;
        Some(Cost::Estimated(u64::try_from(micro).unwrap_or(u64::MAX)))
    }
}

/// Totals for one grouping key (a project, an agent, a day, an account).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Totals {
    pub usage: Usage,
    pub reported_micro_usd: u64,
    pub estimated_micro_usd: u64,
    /// Records for which no cost could be determined (unknown model); shown as "some usage has no price".
    pub unpriced_records: u64,
}

impl Totals {
    pub fn add(&mut self, usage: &Usage, cost: Option<Cost>) {
        self.usage.add(usage);
        match cost {
            Some(Cost::Reported(v)) => {
                self.reported_micro_usd = self.reported_micro_usd.saturating_add(v);
            }
            Some(Cost::Estimated(v)) => {
                self.estimated_micro_usd = self.estimated_micro_usd.saturating_add(v);
            }
            None => self.unpriced_records = self.unpriced_records.saturating_add(1),
        }
    }

    pub fn total_micro_usd(&self) -> u64 {
        self.reported_micro_usd
            .saturating_add(self.estimated_micro_usd)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const CATALOG: &str = r#"{
      "anthropic": { "models": {
        "claude-x": { "cost": { "input": 3, "output": 15, "cache_read": 0.3, "cache_write": 3.75 } },
        "no-price": { "name": "x" }
      } },
      "local": { "models": { "free": { "cost": { "input": 0, "output": 0 } } } },
      "junk": "not an object"
    }"#;

    fn usage(i: u64, o: u64, cr: u64, cw: u64) -> Usage {
        Usage {
            input_tokens: i,
            output_tokens: o,
            cache_read_tokens: cr,
            cache_write_tokens: cw,
        }
    }

    #[test]
    fn reads_prices_and_skips_models_without_one() {
        let c = Catalog::from_models_dev(CATALOG).unwrap();
        assert_eq!(
            c.price("anthropic/claude-x"),
            Some(Price {
                input: 3_000_000,
                output: 15_000_000,
                cache_read: 300_000,
                cache_write: 3_750_000
            })
        );
        assert_eq!(c.price("anthropic/no-price"), None);
        assert_eq!(c.price("local/free").map(|p| p.input), Some(0));
        assert!(Catalog::from_models_dev("not json").is_err());
    }

    #[test]
    fn estimate_is_exact_in_micro_usd() {
        let c = Catalog::from_models_dev(CATALOG).unwrap();
        // 1M input at $3 + 1M output at $15 + 1M cache read at $0.30 + 1M cache write at $3.75 = $22.05
        let cost = c.cost(
            "anthropic/claude-x",
            &usage(1_000_000, 1_000_000, 1_000_000, 1_000_000),
            None,
        );
        assert_eq!(cost, Some(Cost::Estimated(22_050_000)));
        // 1000 input tokens = $0.003 = 3000 micro-USD
        assert_eq!(
            c.cost("anthropic/claude-x", &usage(1000, 0, 0, 0), None)
                .map(Cost::micro_usd),
            Some(3000)
        );
        assert_eq!(
            c.cost("local/free", &usage(5, 5, 0, 0), None),
            Some(Cost::Estimated(0))
        );
    }

    #[test]
    fn reported_cost_wins_and_unknown_models_have_no_estimate() {
        let c = Catalog::from_models_dev(CATALOG).unwrap();
        let u = usage(1_000_000, 0, 0, 0);
        assert_eq!(
            c.cost("anthropic/claude-x", &u, Some(42)),
            Some(Cost::Reported(42))
        );
        assert_eq!(c.cost("mystery/model", &u, None), None);
        // A reported cost needs no price at all.
        assert_eq!(
            c.cost("mystery/model", &u, Some(7)),
            Some(Cost::Reported(7))
        );
    }

    #[test]
    fn overrides_beat_the_catalog() {
        let mut c = Catalog::from_models_dev(CATALOG).unwrap();
        c.set_override(
            "anthropic/claude-x",
            Price {
                input: 1_000_000,
                ..Price::default()
            },
        );
        assert_eq!(
            c.cost(
                "anthropic/claude-x",
                &usage(1_000_000, 1_000_000, 0, 0),
                None
            ),
            Some(Cost::Estimated(1_000_000))
        );
        c.set_override(
            "private/model",
            Price {
                input: 2_000_000,
                output: 0,
                cache_read: 0,
                cache_write: 0,
            },
        );
        assert!(c.price("private/model").is_some());
    }

    #[test]
    fn totals_keep_reported_estimated_and_unpriced_apart() {
        let c = Catalog::from_models_dev(CATALOG).unwrap();
        let mut t = Totals::default();
        let u = usage(1_000_000, 0, 0, 0);
        t.add(&u, c.cost("anthropic/claude-x", &u, None));
        t.add(&u, c.cost("anthropic/claude-x", &u, Some(100)));
        t.add(&u, c.cost("mystery/model", &u, None));
        assert_eq!(t.estimated_micro_usd, 3_000_000);
        assert_eq!(t.reported_micro_usd, 100);
        assert_eq!(t.unpriced_records, 1);
        assert_eq!(t.total_micro_usd(), 3_000_100);
        assert_eq!(t.usage.input_tokens, 3_000_000);
    }

    #[test]
    fn sums_saturate_instead_of_overflowing() {
        let mut t = Totals::default();
        let big = usage(u64::MAX, 0, 0, 0);
        t.add(&big, Some(Cost::Estimated(u64::MAX)));
        t.add(&big, Some(Cost::Estimated(5)));
        assert_eq!(t.usage.input_tokens, u64::MAX);
        assert_eq!(t.estimated_micro_usd, u64::MAX);
    }

    #[test]
    fn a_bare_model_id_uses_its_first_party_price_or_its_only_provider_and_never_a_guess() {
        let c = Catalog::from_models_dev(
            r#"{"anthropic":{"models":{"claude-x":{"cost":{"input":3,"output":15}}}},
                "reseller":{"models":{"claude-x":{"cost":{"input":4,"output":20}},"solo":{"cost":{"input":1,"output":2}},"dup":{"cost":{"input":1,"output":2}}}},
                "other":{"models":{"dup":{"cost":{"input":9,"output":9}}}}}"#,
        )
        .unwrap();
        assert_eq!(
            c.price("claude-x").unwrap().input,
            3_000_000,
            "first party wins"
        );
        assert_eq!(
            c.price("reseller/claude-x").unwrap().input,
            4_000_000,
            "as given"
        );
        assert_eq!(
            c.price("solo").unwrap().output,
            2_000_000,
            "the only provider"
        );
        assert_eq!(c.price("dup"), None, "two resellers, two prices: no guess");
        assert_eq!(c.price(""), None);
    }

    #[test]
    fn the_owners_override_file_beats_the_catalog_and_bad_entries_are_counted() {
        let mut c =
            Catalog::from_models_dev(r#"{"p":{"models":{"m":{"cost":{"input":1,"output":1}}}}}"#)
                .unwrap();
        let skipped = c
            .load_overrides(
                r#"{"p/m":{"input":2.5,"output":10,"cache_read":0.25},"bad":{"input":1}}"#,
            )
            .unwrap();
        assert_eq!(skipped, 1);
        let p = c.price("p/m").unwrap();
        assert_eq!(
            (p.input, p.output, p.cache_read),
            (2_500_000, 10_000_000, 250_000)
        );
        assert!(c.load_overrides("not json").is_err());
    }

    #[test]
    fn the_bundled_table_prices_real_models() {
        assert!(Catalog::bundled()
            .price("anthropic/claude-haiku-4-5")
            .is_some());
    }

    #[test]
    fn f355_a_provider_override_also_wins_when_the_agent_reports_the_bare_model_id() {
        let mut c = Catalog::from_models_dev(
            r#"{"anthropic":{"models":{"shared":{"cost":{"input":3,"output":15}}}}}"#,
        )
        .unwrap();
        c.set_override(
            "anthropic/shared",
            Price {
                input: 1_000_000,
                output: 2_000_000,
                ..Price::default()
            },
        );
        assert_eq!(c.price("shared").unwrap().input, 1_000_000);
    }

    #[test]
    fn an_exact_provider_model_lookup_keeps_using_its_override() {
        let mut c = Catalog::from_models_dev(
            r#"{"anthropic":{"models":{"shared":{"cost":{"input":3,"output":15}}}}}"#,
        )
        .unwrap();
        c.set_override(
            "anthropic/shared",
            Price {
                input: 1_000_000,
                ..Price::default()
            },
        );
        assert_eq!(c.price("anthropic/shared").unwrap().input, 1_000_000);
    }

    #[test]
    fn f356_identical_reseller_prices_are_not_an_ambiguous_cost_guess() {
        let c = Catalog::from_models_dev(
            r#"{"reseller-a":{"models":{"shared":{"cost":{"input":1.5,"output":4}}}},
                "reseller-b":{"models":{"shared":{"cost":{"input":1.5,"output":4}}}}}"#,
        )
        .unwrap();
        assert_eq!(
            c.price("shared"),
            Some(Price {
                input: 1_500_000,
                output: 4_000_000,
                ..Price::default()
            })
        );
    }

    #[test]
    fn different_reseller_prices_remain_ambiguous() {
        let c = Catalog::from_models_dev(
            r#"{"reseller-a":{"models":{"shared":{"cost":{"input":1,"output":4}}}},
                "reseller-b":{"models":{"shared":{"cost":{"input":2,"output":4}}}}}"#,
        )
        .unwrap();
        assert_eq!(c.price("shared"), None);
    }

    #[test]
    fn f357_an_empty_override_key_cannot_price_an_agent_with_no_model() {
        let mut c = Catalog::default();
        c.load_overrides(r#"{"":{"input":1,"output":2}}"#).unwrap();
        assert_eq!(c.price(""), None);
    }

    #[test]
    fn a_named_private_model_override_remains_usable() {
        let mut c = Catalog::default();
        c.load_overrides(r#"{"private/model":{"input":1,"output":2}}"#)
            .unwrap();
        assert_eq!(c.price("private/model").unwrap().output, 2_000_000);
    }

    #[test]
    fn f358_a_price_that_cannot_fit_in_micro_usd_is_not_silently_clamped() {
        let c = Catalog::from_models_dev(
            r#"{"p":{"models":{"too-large":{"cost":{"input":1e300,"output":1}}}}}"#,
        )
        .unwrap();
        assert_eq!(c.price("p/too-large"), None);
    }

    #[test]
    fn a_large_representable_price_remains_usable() {
        let c = Catalog::from_models_dev(
            r#"{"p":{"models":{"large":{"cost":{"input":1000000000000,"output":1}}}}}"#,
        )
        .unwrap();
        assert_eq!(c.price("p/large").unwrap().input, 1_000_000_000_000_000_000);
    }

    #[test]
    fn f359_cost_estimation_saturates_when_all_four_products_overflow_the_u128_sum() {
        let mut c = Catalog::default();
        c.set_override(
            "max/model",
            Price {
                input: u64::MAX,
                output: u64::MAX,
                cache_read: u64::MAX,
                cache_write: u64::MAX,
            },
        );
        assert_eq!(
            c.cost(
                "max/model",
                &usage(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
                None
            ),
            Some(Cost::Estimated(u64::MAX))
        );
    }

    #[test]
    fn one_extreme_cost_product_keeps_saturating_to_the_public_u64_result() {
        let mut c = Catalog::default();
        c.set_override(
            "max/model",
            Price {
                input: u64::MAX,
                ..Price::default()
            },
        );
        assert_eq!(
            c.cost("max/model", &usage(u64::MAX, 0, 0, 0), None),
            Some(Cost::Estimated(u64::MAX))
        );
    }

    #[test]
    fn verifies_f355_bare_lookup_overrides_for_first_party_sole_and_competing_providers() {
        let mut c = Catalog::from_models_dev(
            r#"{"anthropic":{"models":{"first":{"cost":{"input":3,"output":3}},"shared":{"cost":{"input":4,"output":4}}}},
                "reseller":{"models":{"solo":{"cost":{"input":5,"output":5}},"shared":{"cost":{"input":6,"output":6}}}}}"#,
        )
        .unwrap();
        for (model, input) in [
            ("anthropic/first", 1_000_000),
            ("reseller/solo", 2_000_000),
            ("anthropic/shared", 3_000_000),
        ] {
            c.set_override(
                model,
                Price {
                    input,
                    ..Price::default()
                },
            );
        }
        assert_eq!(c.price("first").unwrap().input, 1_000_000);
        assert_eq!(c.price("solo").unwrap().input, 2_000_000);
        assert_eq!(c.price("shared").unwrap().input, 3_000_000);
    }

    #[test]
    fn verifies_f356_identical_prices_across_two_three_and_zero_cost_providers() {
        for raw in [
            r#"{"a":{"models":{"m":{"cost":{"input":1,"output":2}}}},"b":{"models":{"m":{"cost":{"input":1,"output":2}}}}}"#,
            r#"{"a":{"models":{"m":{"cost":{"input":0.25,"output":0.5}}}},"b":{"models":{"m":{"cost":{"input":0.25,"output":0.5}}}},"c":{"models":{"m":{"cost":{"input":0.25,"output":0.5}}}}}"#,
            r#"{"a":{"models":{"m":{"cost":{"input":0,"output":0}}}},"b":{"models":{"m":{"cost":{"input":0,"output":0}}}}}"#,
        ] {
            let c = Catalog::from_models_dev(raw).unwrap();
            assert!(c.price("m").is_some(), "{raw}");
        }
    }

    #[test]
    fn verifies_f357_empty_models_stay_unpriced_across_direct_file_and_cost_lookups() {
        let mut direct = Catalog::default();
        direct.set_override(
            "",
            Price {
                input: 1,
                output: 2,
                ..Price::default()
            },
        );
        assert_eq!(direct.price(""), None);

        let mut file = Catalog::default();
        file.load_overrides(r#"{"":{"input":3,"output":4}}"#)
            .unwrap();
        assert_eq!(file.price(""), None);
        assert_eq!(
            file.cost("", &usage(1_000_000, 1_000_000, 0, 0), None),
            None
        );
    }

    #[test]
    fn verifies_f358_out_of_range_prices_across_integer_decimal_and_exponent_forms() {
        for raw in ["1e300", "18446744073709.552", "100000000000000000000"] {
            let value: Value = serde_json::from_str(raw).unwrap();
            assert_eq!(micro(&value), None, "{raw}");
        }
        let value: Value = serde_json::from_str("1000000000000").unwrap();
        assert_eq!(micro(&value), Some(1_000_000_000_000_000_000));
    }

    #[test]
    fn verifies_f359_saturation_across_two_four_and_near_max_product_sums() {
        let mut c = Catalog::default();
        c.set_override(
            "max/model",
            Price {
                input: u64::MAX,
                output: u64::MAX,
                cache_read: u64::MAX,
                cache_write: u64::MAX,
            },
        );
        for usage in [
            usage(u64::MAX, u64::MAX, 0, 0),
            usage(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
            usage(u64::MAX - 1, u64::MAX - 1, u64::MAX - 1, 0),
        ] {
            assert_eq!(
                c.cost("max/model", &usage, None),
                Some(Cost::Estimated(u64::MAX))
            );
        }
    }

    #[test]
    fn f360_the_unpriced_record_counter_saturates_instead_of_overflowing() {
        let mut totals = Totals {
            unpriced_records: u64::MAX,
            ..Totals::default()
        };
        totals.add(&Usage::default(), None);
        assert_eq!(totals.unpriced_records, u64::MAX);
    }

    #[test]
    fn the_unpriced_record_counter_still_reaches_its_maximum() {
        let mut totals = Totals {
            unpriced_records: u64::MAX - 1,
            ..Totals::default()
        };
        totals.add(&Usage::default(), None);
        assert_eq!(totals.unpriced_records, u64::MAX);
    }

    #[test]
    fn f361_decimal_prices_round_exactly_at_a_half_micro_usd() {
        let c = Catalog::from_models_dev(
            r#"{"p":{"models":{"half":{"cost":{"input":0.0001245,"output":0.0001245}}}}}"#,
        )
        .unwrap();
        assert_eq!(c.price("p/half").unwrap().input, 125);
    }

    #[test]
    fn a_decimal_price_below_the_half_micro_usd_still_rounds_down() {
        let c = Catalog::from_models_dev(
            r#"{"p":{"models":{"low":{"cost":{"input":0.0001244,"output":0.0001244}}}}}"#,
        )
        .unwrap();
        assert_eq!(c.price("p/low").unwrap().input, 124);
    }

    #[test]
    fn f362_a_nested_model_id_is_not_mistaken_for_its_last_path_segment() {
        let c = Catalog::from_models_dev(
            r#"{"reseller":{"models":{"org/shared":{"cost":{"input":1,"output":2}}}}}"#,
        )
        .unwrap();
        assert_eq!(c.price("shared"), None);
    }

    #[test]
    fn a_nested_model_id_remains_available_by_its_complete_catalog_key() {
        let c = Catalog::from_models_dev(
            r#"{"reseller":{"models":{"org/shared":{"cost":{"input":1,"output":2}}}}}"#,
        )
        .unwrap();
        assert_eq!(c.price("reseller/org/shared").unwrap().input, 1_000_000);
    }

    #[test]
    fn f363_a_present_but_invalid_cache_price_does_not_become_a_free_cache_price() {
        let c = Catalog::from_models_dev(
            r#"{"p":{"models":{"bad-cache":{"cost":{"input":1,"output":2,"cache_read":-1}}}}}"#,
        )
        .unwrap();
        assert_eq!(c.price("p/bad-cache"), None);
    }

    #[test]
    fn a_missing_cache_price_still_means_no_separate_cache_charge() {
        let c = Catalog::from_models_dev(
            r#"{"p":{"models":{"no-cache":{"cost":{"input":1,"output":2}}}}}"#,
        )
        .unwrap();
        assert_eq!(c.price("p/no-cache").unwrap().cache_read, 0);
    }

    #[test]
    fn f364_a_unique_provider_override_can_price_the_usual_bare_model_id() {
        let mut c = Catalog::default();
        c.set_override(
            "private/solo",
            Price {
                input: 7,
                output: 9,
                ..Price::default()
            },
        );
        assert_eq!(c.price("solo").unwrap().input, 7);
    }

    #[test]
    fn a_provider_override_remains_available_by_its_complete_key() {
        let mut c = Catalog::default();
        c.set_override(
            "private/solo",
            Price {
                input: 7,
                ..Price::default()
            },
        );
        assert_eq!(c.price("private/solo").unwrap().input, 7);
    }

    #[test]
    fn verifies_f360_unpriced_counts_across_normal_near_max_and_saturated_totals() {
        for (starting, additions, expected) in [
            (0, 3, 3),
            (u64::MAX - 1, 1, u64::MAX),
            (u64::MAX, 2, u64::MAX),
        ] {
            let mut totals = Totals {
                unpriced_records: starting,
                ..Totals::default()
            };
            for _ in 0..additions {
                totals.add(&Usage::default(), None);
            }
            assert_eq!(totals.unpriced_records, expected);
        }
    }

    #[test]
    fn verifies_f361_exact_rounding_across_tiny_decimal_scientific_and_below_half_prices() {
        for (raw, expected) in [
            ("0.0000005", 1),
            ("0.0001245", 125),
            ("1.245e-4", 125),
            ("0.0001244999", 124),
        ] {
            let value: Value = serde_json::from_str(raw).unwrap();
            assert_eq!(micro(&value), Some(expected), "{raw}");
        }
    }

    #[test]
    fn verifies_f362_nested_ids_across_one_many_and_zero_inner_slashes() {
        let c = Catalog::from_models_dev(
            r#"{"reseller":{"models":{
                "org/shared":{"cost":{"input":1,"output":2}},
                "a/b/deep":{"cost":{"input":3,"output":4}},
                "plain":{"cost":{"input":5,"output":6}}
            }}}"#,
        )
        .unwrap();
        assert_eq!(c.price("shared"), None);
        assert_eq!(c.price("deep"), None);
        assert_eq!(c.price("reseller/org/shared").unwrap().input, 1_000_000);
        assert_eq!(c.price("reseller/a/b/deep").unwrap().input, 3_000_000);
        assert_eq!(c.price("plain").unwrap().input, 5_000_000);
    }

    #[test]
    fn verifies_f363_invalid_cache_prices_across_negative_non_numeric_and_overflow_values() {
        let c = Catalog::from_models_dev(
            r#"{"p":{"models":{
                "negative":{"cost":{"input":1,"output":2,"cache_read":-1}},
                "text":{"cost":{"input":1,"output":2,"cache_write":"free"}},
                "overflow":{"cost":{"input":1,"output":2,"cache_read":1e300}},
                "missing":{"cost":{"input":1,"output":2}}
            }}}"#,
        )
        .unwrap();
        for model in ["negative", "text", "overflow"] {
            assert_eq!(c.price(&format!("p/{model}")), None, "{model}");
        }
        assert_eq!(c.price("p/missing").unwrap().cache_read, 0);
    }

    #[test]
    fn verifies_f364_override_only_bare_lookups_for_unique_equal_ambiguous_and_first_party_prices()
    {
        let mut c = Catalog::default();
        for (model, input) in [
            ("private/unique", 1),
            ("reseller-a/equal", 2),
            ("reseller-b/equal", 2),
            ("reseller-a/ambiguous", 3),
            ("reseller-b/ambiguous", 4),
            ("openai/first-party", 5),
            ("reseller/first-party", 6),
        ] {
            c.set_override(
                model,
                Price {
                    input,
                    ..Price::default()
                },
            );
        }
        assert_eq!(c.price("unique").unwrap().input, 1);
        assert_eq!(c.price("equal").unwrap().input, 2);
        assert_eq!(c.price("ambiguous"), None);
        assert_eq!(c.price("first-party").unwrap().input, 5);
    }
}
