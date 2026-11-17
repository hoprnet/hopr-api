//! Validated probability values for graph observations.

/// A value in the closed interval `[0.0, 1.0]`.
///
/// The interval is closed: `0.0` and `1.0` are both valid and carry meaning. Zero is a real
/// observation or setting, such as a measured-dead probe score, a zero acknowledgment rate, a zero
/// penalty, or an unusable relayer. One is a fully healthy or fully acknowledging one. `-0.0`
/// compares equal to zero and is accepted.
///
/// [`new`](Self::new) and [`TryFrom<f64>`] reject `NaN`, both infinities, and every value outside
/// the interval, so a `Probability` needs no clamp and no NaN check downstream. Deserialization
/// behind the `serde` feature validates the same way; serialization stays a plain `f64`.
///
/// The accumulated path value is deliberately not a `Probability`: it carries negative rejection
/// sentinels and can exceed one.
///
/// ```
/// use hopr_api::graph::Probability;
///
/// let half = Probability::new(0.5)?;
/// assert_eq!(half.get(), 0.5);
/// assert!(Probability::try_from(1.5).is_err());
/// # Ok::<(), hopr_api::graph::ProbabilityError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Probability(f64);

/// Error returned when a value does not lie in the closed interval `[0.0, 1.0]`.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("value {0} is not a probability in the closed interval [0.0, 1.0]")]
pub struct ProbabilityError(f64);

impl Probability {
    /// One, the inclusive upper endpoint.
    ///
    /// Fully healthy relayer or full acknowledgment rate.
    pub const ONE: Self = Self(1.0);
    /// Zero, the inclusive lower endpoint.
    ///
    /// Unusable relayer, measured-dead stream, or zero acknowledgment rate.
    pub const ZERO: Self = Self(0.0);

    /// Creates a probability from `value`, rejecting anything outside `[0.0, 1.0]`.
    ///
    /// Accepts `-0.0` because it compares equal to zero, and normalizes it to `0.0`. Rejects
    /// `NaN` and both infinities.
    pub const fn new(value: f64) -> Result<Self, ProbabilityError> {
        if value >= 0.0 && value <= 1.0 {
            // Adding zero folds `-0.0` into `0.0` so the representation is unique.
            Ok(Self(value + 0.0))
        } else {
            Err(ProbabilityError(value))
        }
    }

    /// Returns the midpoint of `self` and `other`.
    ///
    /// The midpoint of two values in `[0.0, 1.0]` is itself in the range, so no validation applies.
    pub const fn avg(self, other: Self) -> Self {
        Self((self.0 + other.0) / 2.0)
    }

    /// Returns the underlying `f64`, guaranteed to lie in `[0.0, 1.0]`.
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Probability {
    type Error = ProbabilityError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Probability> for f64 {
    fn from(probability: Probability) -> Self {
        probability.0
    }
}

// The in-crate snapshot fixtures serialize their observation fields, so test builds need these
// impls even when the `serde` feature is disabled.
#[cfg(any(feature = "serde", test))]
impl serde::Serialize for Probability {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_f64(self.0)
    }
}

#[cfg(any(feature = "serde", test))]
impl<'de> serde::Deserialize<'de> for Probability {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <f64 as serde::Deserialize>::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use rstest::rstest;

    use super::*;

    /// Builds a [`Probability`] for fixtures, panicking on an out-of-range literal.
    const fn probability(value: f64) -> Probability {
        match Probability::new(value) {
            Ok(probability) => probability,
            Err(_) => panic!("test fixture must be a valid probability"),
        }
    }

    #[rstest]
    #[case::zero(0.0)]
    #[case::one(1.0)]
    #[case::mid_range(0.5)]
    #[case::measured_dead_floor(1e-9)]
    #[case::almost_one(0.999_999)]
    #[case::negative_zero(-0.0)]
    fn probability_should_accept_values_inside_the_closed_interval(#[case] value: f64) -> anyhow::Result<()> {
        let probability = Probability::new(value).context("a value in the interval must be accepted")?;
        assert_eq!(probability.get(), value);
        assert_eq!(
            Probability::try_from(value).context("TryFrom must accept the same values")?,
            probability
        );
        Ok(())
    }

    #[rstest]
    #[case::nan(f64::NAN)]
    #[case::positive_infinity(f64::INFINITY)]
    #[case::negative_infinity(f64::NEG_INFINITY)]
    #[case::just_below_zero(-0.1)]
    #[case::just_above_one(1.1)]
    #[case::far_below_zero(-12323.0)]
    #[case::far_above_one(12323.0)]
    fn probability_should_reject_values_outside_the_closed_interval(#[case] value: f64) {
        assert!(
            matches!(Probability::new(value), Err(ProbabilityError(_))),
            "must reject {value}"
        );
        assert!(
            matches!(Probability::try_from(value), Err(ProbabilityError(_))),
            "TryFrom must reject {value}"
        );
    }

    #[rstest]
    #[case::zeros(Probability::ZERO, Probability::ZERO, 0.0)]
    #[case::endpoints(Probability::ZERO, Probability::ONE, 0.5)]
    #[case::ones(Probability::ONE, Probability::ONE, 1.0)]
    #[case::same_degraded(probability(0.25), probability(0.25), 0.25)]
    #[case::degraded_pair(probability(0.25), probability(0.75), 0.5)]
    fn probability_should_average_within_the_interval(
        #[case] left: Probability,
        #[case] right: Probability,
        #[case] expected: f64,
    ) {
        let midpoint = left.avg(right);
        assert_eq!(midpoint, right.avg(left), "the midpoint must be symmetric");
        assert!(
            (midpoint.get() - expected).abs() < 1e-12,
            "expected {expected}, got {}",
            midpoint.get()
        );
    }

    #[test]
    fn probability_should_convert_into_a_plain_f64() {
        assert_eq!(f64::from(Probability::ONE), 1.0);
        assert_eq!(f64::from(Probability::ZERO), 0.0);
    }

    #[cfg(feature = "serde")]
    mod serde_contract {
        use anyhow::Context;
        use rstest::rstest;

        use super::*;

        #[test]
        fn probability_should_serialize_as_a_bare_f64() -> anyhow::Result<()> {
            let probability = Probability::new(0.25).context("0.25 is valid")?;
            assert_eq!(
                serde_json::to_string(&probability).context("serialization must succeed")?,
                "0.25"
            );
            Ok(())
        }

        #[test]
        fn probability_should_round_trip_through_a_bare_f64() -> anyhow::Result<()> {
            let probability = Probability::new(0.625).context("0.625 is valid")?;
            let json = serde_json::to_string(&probability).context("serialization must succeed")?;
            let restored: Probability = serde_json::from_str(&json).context("deserialization must succeed")?;
            assert_eq!(restored, probability);
            Ok(())
        }

        #[rstest]
        #[case::zero("0.0")]
        #[case::one("1.0")]
        fn probability_should_deserialize_its_endpoints(#[case] json: &str) -> anyhow::Result<()> {
            let probability: Probability = serde_json::from_str(json).context("endpoints must deserialize")?;
            assert_eq!(
                probability.get(),
                json.parse::<f64>().context("the case is a valid f64")?
            );
            Ok(())
        }

        #[rstest]
        #[case::below_zero("-0.5")]
        #[case::above_one("1.5")]
        fn probability_should_reject_out_of_interval_deserialization(#[case] json: &str) {
            assert!(
                serde_json::from_str::<Probability>(json).is_err(),
                "deserialization must reject {json}"
            );
        }
    }
}
