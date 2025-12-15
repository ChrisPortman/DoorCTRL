pub(crate) const CR: u8 = 13;
pub(crate) const LF: u8 = 10;
pub(crate) const SP: u8 = 32;
pub(crate) const COLON: u8 = 58;
pub(crate) const ZERO: u8 = 48;

pub(crate) fn atoi(data: &[u8]) -> Option<u32> {
    let mut val: u32 = 0;

    let len: u32 = match data.len().try_into() {
        Ok(n) => n,
        Err(_) => return None,
    };

    for (i, digit) in data.iter().enumerate() {
        if *digit < 48 || *digit > 57 {
            return None;
        }

        let possition: u32 = match TryInto::<u32>::try_into(i) {
            Ok(n) => n + 1,
            Err(_) => return None,
        };

        let exp = len - possition;

        let digit_val: u32 = (digit - 48).into();
        val += digit_val * 10_u32.pow(exp);
    }

    Some(val)
}

pub(crate) struct AsciiInt([u8; 20]);

impl AsciiInt {
    pub(crate) fn as_str(&self) -> &str {
        str::from_utf8(&self.0).unwrap().trim()
    }
}

impl From<u64> for AsciiInt {
    fn from(value: u64) -> Self {
        let divmod10 = |d: u64| -> (u64, u8) {
            let int = d / 10;
            let rem = d % 10;
            (int, rem.try_into().unwrap())
        };

        let mut round = 0;
        let mut int = value;
        let mut rem: u8;

        let mut ret_array = [SP; 20];
        loop {
            (int, rem) = divmod10(int);
            ret_array[19 - round] = rem + ZERO;
            if int == 0 {
                break;
            }
            round += 1;
        }

        AsciiInt(ret_array)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn test_atoi() {
        assert!(atoi("0".as_bytes()) == Some(0));
        assert!(atoi("5".as_bytes()) == Some(5));
        assert!(atoi("123".as_bytes()) == Some(123));
        assert!(atoi("123456789".as_bytes()) == Some(123456789));
        assert!(atoi("0123456789".as_bytes()) == Some(123456789));
        assert!(atoi("abc".as_bytes()) == None);
        assert!(atoi("123a456".as_bytes()) == None);
    }

    #[test]
    fn test_itoa() {
        let a: AsciiInt = 1u64.into();
        assert!("1" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 12u64.into();
        assert!("12" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 123u64.into();
        assert!("123" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 1203u64.into();
        assert!("1203" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 12030u64.into();
        assert!("12030" == a.as_str(), "got: {:?}", a.as_str());
        let a: AsciiInt = 100002u64.into();
        assert!("100002" == a.as_str(), "got: {:?}", a.as_str());
    }
}
