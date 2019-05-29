use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::str::FromStr;

mod precision;
use self::precision::*;

pub mod error;
pub use self::error::*;

pub const MAX_STR_LEN: usize = 26;
pub const MAX_PRECISION: u8 = 4;

#[derive(Copy, Clone, Debug, Default, PartialEq, PartialOrd)]
pub struct Asset {
    pub amount: i64,
}

impl Asset {
    #[inline]
    pub const fn new(amount: i64) -> Asset {
        Asset { amount }
    }

    #[inline]
    pub fn add(&self, other: Self) -> Option<Self> {
        Some(Asset {
            amount: self.amount.checked_add(other.amount)?,
        })
    }

    #[inline]
    pub fn sub(&self, other: Self) -> Option<Self> {
        Some(Asset {
            amount: self.amount.checked_sub(other.amount)?,
        })
    }

    pub fn mul(&self, other: Self) -> Option<Self> {
        const MUL_PRECISION: u8 = MAX_PRECISION * 2;
        let mul = i128::from(self.amount).checked_mul(i128::from(other.amount))?;
        let final_mul = set_decimals_i128(mul, MUL_PRECISION, MAX_PRECISION)?;
        if final_mul > i128::from(::std::i64::MAX) {
            return None;
        }
        Some(Asset {
            amount: final_mul as i64,
        })
    }

    pub fn div(&self, other: Self) -> Option<Self> {
        if other.amount == 0 {
            return None;
        }
        const DIV_PRECISION: u8 = MAX_PRECISION * 2;
        let a = set_decimals_i64(self.amount, MAX_PRECISION, DIV_PRECISION)?;
        Some(Asset {
            amount: a.checked_div(other.amount)?,
        })
    }

    pub fn pow(&self, num: u16) -> Option<Self> {
        if num == 0 {
            return Some(Asset {
                amount: set_decimals_i64(1, 0, MAX_PRECISION)?,
            });
        }

        let decimals = u16::from(MAX_PRECISION).checked_mul(num)?;
        let mut res = BigInt::from(1);
        {
            let mut base = BigInt::from(self.amount);
            let mut exp = num;
            loop {
                if exp & 1 == 1 {
                    res = &res * &base;
                }
                exp >>= 1;
                if exp == 0 {
                    break;
                };
                base = &base * &base;
            }
        }

        res = set_decimals_big(&res, decimals, u16::from(MAX_PRECISION));
        Some(Asset {
            amount: res.to_i64()?,
        })
    }
}

impl ToString for Asset {
    fn to_string(&self) -> String {
        let mut s = self.amount.to_string();
        {
            let len = s.len();
            if len < MAX_PRECISION as usize {
                let start = if self.amount < 0 { 1 } else { 0 };
                let diff = MAX_PRECISION as usize - len + start;
                s.insert_str(start, "0.");
                s.insert_str(start + 2, &"0".repeat(diff));
            } else if len == MAX_PRECISION as usize {
                s.insert_str(0, "0.");
            } else {
                s.insert(len - (MAX_PRECISION as usize), '.');
            }
        }
        s.push_str(" GRAEL");
        s
    }
}

impl FromStr for Asset {
    type Err = AssetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > MAX_STR_LEN {
            return Err(AssetError {
                kind: AssetErrorKind::StrTooLarge,
            });
        }
        let mut split = s.trim().splitn(2, ' ');

        let amount: i64;
        match split.next() {
            Some(x) => {
                match x.find('.') {
                    Some(pos) => {
                        // Check decimal precision is correct
                        {
                            let decimals = {
                                let len = x.len() - 1;
                                if pos > 0 {
                                    (len - pos) as u8
                                } else {
                                    len as u8
                                }
                            };

                            if decimals != MAX_PRECISION {
                                return Err(AssetError {
                                    kind: AssetErrorKind::InvalidAmount,
                                });
                            }
                        }

                        // Actually parse the amount
                        amount = match x.replace('.', "").parse() {
                            Ok(x) => x,
                            Err(_) => {
                                return Err(AssetError {
                                    kind: AssetErrorKind::InvalidAmount,
                                });
                            }
                        }
                    }
                    None => {
                        return Err(AssetError {
                            kind: AssetErrorKind::InvalidAmount,
                        });
                    }
                }
            }
            None => {
                return Err(AssetError {
                    kind: AssetErrorKind::InvalidFormat,
                });
            }
        };

        match split.next() {
            Some(x) => {
                if x != "GRAEL" {
                    return Err(AssetError {
                        kind: AssetErrorKind::InvalidAssetType,
                    });
                }
            }
            None => {
                return Err(AssetError {
                    kind: AssetErrorKind::InvalidFormat,
                });
            }
        };

        Ok(Asset { amount })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_input() {
        let c = |asset: Asset, amount: &str| {
            assert_eq!(asset.amount.to_string(), amount);
        };

        c(get_asset("1.0000 GRAEL"), "10000");
        c(get_asset("-1.0000 GRAEL"), "-10000");
        c(get_asset(".1000 GRAEL"), "1000");
        c(get_asset("-.1000 GRAEL"), "-1000");
        c(get_asset("0.1000 GRAEL"), "1000");
        c(get_asset("0.0000 GRAEL"), "0");
        c(get_asset("-0.0000 GRAEL"), "0");
    }

    #[test]
    fn test_asset_to_str() {
        let c = |asset: Asset, s: &str| {
            assert_eq!(asset.to_string(), s);
        };
        c(get_asset("1.0001 GRAEL"), "1.0001 GRAEL");
        c(get_asset("0.0001 GRAEL"), "0.0001 GRAEL");
        c(get_asset("-0.0001 GRAEL"), "-0.0001 GRAEL");
        c(get_asset(".0001 GRAEL"), "0.0001 GRAEL");
        c(get_asset(".1000 GRAEL"), "0.1000 GRAEL");
        c(get_asset("1.0000 GRAEL"), "1.0000 GRAEL");
    }

    #[test]
    fn test_fail_parsing_invalid_input() {
        let c = |asset: &str, err: AssetErrorKind| {
            let e = Asset::from_str(asset).err().unwrap();
            assert_eq!(e.kind, err);
        };

        c("1e10 GRAEL", AssetErrorKind::InvalidAmount);
        c("a100 GRAEL", AssetErrorKind::InvalidAmount);
        c("100a GRAEL", AssetErrorKind::InvalidAmount);

        c("1 GRAEL", AssetErrorKind::InvalidAmount);
        c("1. GRAEL", AssetErrorKind::InvalidAmount);
        c(".1 GRAEL", AssetErrorKind::InvalidAmount);
        c("-.1 GRAEL", AssetErrorKind::InvalidAmount);
        c("0.1 GRAEL", AssetErrorKind::InvalidAmount);
        c("1.0 GRAEL", AssetErrorKind::InvalidAmount);
        c("0 GRAEL", AssetErrorKind::InvalidAmount);
        c("-0.0 GRAEL", AssetErrorKind::InvalidAmount);
        c("-1.0 GRAEL", AssetErrorKind::InvalidAmount);
        c("1.00000 GRAEL", AssetErrorKind::InvalidAmount);

        c(
            "1234567890123456789012345678 GRAEL",
            AssetErrorKind::StrTooLarge,
        );
        c("1.0000", AssetErrorKind::InvalidFormat);

        c("1.0000 GRAEL a", AssetErrorKind::InvalidAssetType);
        c("1.0000 grael", AssetErrorKind::InvalidAssetType);
    }

    #[test]
    fn test_perform_arithmetic() {
        let c = |asset: Asset, amount: &str| {
            assert_eq!(asset.to_string(), amount);
        };

        let a = get_asset("123.4560 GRAEL");
        c(a.add(get_asset("2.0000 GRAEL")).unwrap(), "125.4560 GRAEL");
        c(a.add(get_asset("-2.0000 GRAEL")).unwrap(), "121.4560 GRAEL");
        c(a.add(get_asset(".0001 GRAEL")).unwrap(), "123.4561 GRAEL");
        c(a.sub(get_asset("2.0000 GRAEL")).unwrap(), "121.4560 GRAEL");
        c(a.sub(get_asset("-2.0000 GRAEL")).unwrap(), "125.4560 GRAEL");
        c(
            a.mul(get_asset("100000.1111 GRAEL")).unwrap(),
            "12345613.7159 GRAEL",
        );
        c(
            a.mul(get_asset("-100000.1111 GRAEL")).unwrap(),
            "-12345613.7159 GRAEL",
        );
        c(a.div(get_asset("23.0000 GRAEL")).unwrap(), "5.3676 GRAEL");
        c(a.div(get_asset("-23.0000 GRAEL")).unwrap(), "-5.3676 GRAEL");
        c(a.pow(2).unwrap(), "15241.3839 GRAEL");
        c(a.pow(3).unwrap(), "1881640.2952 GRAEL");
        c(a, "123.4560 GRAEL");

        c(get_asset("1.0002 GRAEL").pow(1000).unwrap(), "1.2213 GRAEL");
        c(
            get_asset("10.0000 GRAEL")
                .div(get_asset("2.0000 GRAEL"))
                .unwrap(),
            "5.0000 GRAEL",
        );
        c(
            get_asset("5.0000 GRAEL")
                .div(get_asset("10.0000 GRAEL"))
                .unwrap(),
            "0.5000 GRAEL",
        );

        assert!(a.div(get_asset("0.0000 GRAEL")).is_none());
    }

    #[test]
    fn test_invalid_arithmetic() {
        let a = get_asset("10.0000 GRAEL");
        let b = get_asset("922337203685477.5807 GRAEL");

        assert_eq!(a.add(b), None);
        assert_eq!(a.mul(Asset::new(-1)).unwrap().sub(b), None);
        assert_eq!(a.div(Asset::new(0)), None);
        assert_eq!(a.mul(b), None);
    }

    fn get_asset(s: &str) -> Asset {
        Asset::from_str(s).unwrap()
    }
}
