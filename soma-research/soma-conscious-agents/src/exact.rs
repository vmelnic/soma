use anyhow::{bail, Result};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, ToPrimitive, Zero};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

/// An arbitrary-precision rational used by every stochastic law and certificate.
/// No floating-point comparison participates in formal validation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct Exact(pub BigRational);

impl Exact {
    pub fn new(numerator: i128, denominator: i128) -> Result<Self> {
        if denominator == 0 {
            bail!("an exact denominator cannot be zero");
        }
        Ok(Self(BigRational::new(
            BigInt::from(numerator),
            BigInt::from(denominator),
        )))
    }

    pub fn from_counts(numerator: usize, denominator: usize) -> Result<Self> {
        if denominator == 0 {
            bail!("a count denominator cannot be zero");
        }
        Ok(Self(BigRational::new(
            BigInt::from(numerator),
            BigInt::from(denominator),
        )))
    }

    pub fn from_u64(value: u64) -> Self {
        Self(BigRational::from_integer(BigInt::from(value)))
    }

    /// Exact prefix-code mass `2^-exponent`. This is used for finite
    /// description-length priors without introducing floating-point scores.
    pub fn inverse_power_of_two(exponent: usize) -> Self {
        Self(BigRational::new(BigInt::one(), BigInt::one() << exponent))
    }

    pub fn zero() -> Self {
        Self(BigRational::zero())
    }

    pub fn one() -> Self {
        Self(BigRational::one())
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    pub fn is_one(&self) -> bool {
        self.0.is_one()
    }

    pub fn is_negative(&self) -> bool {
        self.0 < BigRational::zero()
    }

    pub fn add(&self, rhs: &Self) -> Self {
        Self(self.0.clone() + rhs.0.clone())
    }

    pub fn sub(&self, rhs: &Self) -> Self {
        Self(self.0.clone() - rhs.0.clone())
    }

    pub fn mul(&self, rhs: &Self) -> Self {
        Self(self.0.clone() * rhs.0.clone())
    }

    pub fn div(&self, rhs: &Self) -> Result<Self> {
        if rhs.is_zero() {
            bail!("division by zero");
        }
        Ok(Self(self.0.clone() / rhs.0.clone()))
    }

    pub fn reciprocal(&self) -> Result<Self> {
        Self::one().div(self)
    }

    pub fn to_f64(&self) -> Result<f64> {
        self.0
            .to_f64()
            .ok_or_else(|| anyhow::anyhow!("exact value cannot be represented as f64"))
    }

    pub fn numerator(&self) -> String {
        self.0.numer().to_string()
    }

    pub fn denominator(&self) -> String {
        self.0.denom().to_string()
    }
}

impl Default for Exact {
    fn default() -> Self {
        Self::zero()
    }
}

impl Display for Exact {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        if self.0.denom().is_one() {
            write!(formatter, "{}", self.0.numer())
        } else {
            write!(formatter, "{}/{}", self.0.numer(), self.0.denom())
        }
    }
}

pub fn sum(values: impl IntoIterator<Item = Exact>) -> Exact {
    values
        .into_iter()
        .fold(Exact::zero(), |total, value| total.add(&value))
}
