//! EXSLT dates and times (`http://exslt.org/dates-and-times`), ported from
//! libexslt 0.8.20's `date.c`: the current date and time, and the functions
//! that take a date or time apart. DocBook dates its man pages with them.
//!
//! The parser fills in one value field by field, as `exsltDateParse` does,
//! including its quirks: a failed time parse falls back to a date, and a
//! failed time-zone parse can leave a partial offset behind.
//!
//! `date:add`, `date:add-duration`, `date:difference`, `date:duration`,
//! `date:seconds` and `date:sum` aren't ported yet.
//!
//! The current time honours `SOURCE_DATE_EPOCH`, as libexslt does for
//! reproducible builds, and is then in UTC. Without it libexslt uses the
//! local time zone; oxslt uses UTC, since reading the local zone needs the C
//! library (a known difference when the machine's zone isn't UTC).

use crate::xpath::{self, Env, Value, XPathError};

// Date and time types, from least to most specific (exsltDateType).
const UNKNOWN: u8 = 0;
const TIME: u8 = 1;
const GDAY: u8 = TIME << 1;
const GMONTH: u8 = GDAY << 1;
const GMONTHDAY: u8 = GMONTH | GDAY;
const GYEAR: u8 = GMONTH << 1;
const GYEARMONTH: u8 = GYEAR | GMONTH;
const DATE: u8 = GYEAR | GMONTH | GDAY;
const DATETIME: u8 = DATE | TIME;

/// The functions ported so far.
pub const IMPLEMENTED: &[&str] = &[
    "date-time",
    "date",
    "time",
    "year",
    "leap-year",
    "month-in-year",
    "month-name",
    "month-abbreviation",
    "week-in-year",
    "week-in-month",
    "day-in-year",
    "day-in-month",
    "day-of-week-in-month",
    "day-in-week",
    "day-name",
    "day-abbreviation",
    "hour-in-day",
    "minute-in-hour",
    "second-in-minute",
];

/// exsltDateVal.
#[derive(Clone, Copy, Debug)]
struct DateVal {
    ty: u8,
    /// Internally continuous: 0 is 1 BC, -1 is 2 BC.
    year: i64,
    mon: u32,
    day: u32,
    hour: u32,
    min: u32,
    sec: f64,
    tz_flag: bool,
    /// Minutes east of UTC.
    tzo: i32,
}

impl DateVal {
    /// exsltDateCreateDate: zeroed, with month and day 1.
    fn new(ty: u8) -> DateVal {
        DateVal {
            ty,
            year: 0,
            mon: 1,
            day: 1,
            hour: 0,
            min: 0,
            sec: 0.0,
            tz_flag: false,
            tzo: 0,
        }
    }
}

fn is_leap(y: i64) -> bool {
    (y & 3) == 0 && (y % 25 != 0 || (y & 15) == 0)
}

const DAYS_IN_MONTH: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
const DAYS_IN_MONTH_LEAP: [i64; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
const DAY_IN_YEAR_BY_MONTH: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
const DAY_IN_LEAP_YEAR_BY_MONTH: [i64; 12] =
    [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335];

fn day_in_year(day: i64, month: u32, year: i64) -> i64 {
    let table = if is_leap(year) {
        &DAY_IN_LEAP_YEAR_BY_MONTH
    } else {
        &DAY_IN_YEAR_BY_MONTH
    };
    table[month as usize - 1] + day
}

fn valid_date(dt: &DateVal) -> bool {
    (1..=12).contains(&dt.mon) && {
        let days = if is_leap(dt.year) {
            &DAYS_IN_MONTH_LEAP
        } else {
            &DAYS_IN_MONTH
        };
        i64::from(dt.day) <= days[dt.mon as usize - 1]
    }
}

fn valid_time(dt: &DateVal) -> bool {
    dt.hour <= 23
        && dt.min <= 59
        && (0.0..60.0).contains(&dt.sec)
        && dt.tzo > -1440
        && dt.tzo < 1440
}

fn valid_datetime(dt: &DateVal) -> bool {
    valid_date(dt) && valid_time(dt)
}

/// A byte of the string, 0 past its end (as C's terminator).
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

fn digit(c: u8) -> bool {
    c.is_ascii_digit()
}

/// PARSE_2_DIGITS: 0, or 1 if not two digits, or 2 if `valid` rejects the
/// number. `cur` moves past the two characters either way.
fn parse_2_digits(s: &[u8], cur: &mut usize, valid: fn(u32) -> bool, num: &mut u32) -> i32 {
    let (a, b) = (at(s, *cur), at(s, *cur + 1));
    let ret = if !digit(a) || !digit(b) {
        1
    } else {
        let val = u32::from(a - b'0') * 10 + u32::from(b - b'0');
        if valid(val) {
            *num = val;
            0
        } else {
            2
        }
    };
    *cur += 2;
    ret
}

/// _exsltDateParseGYear.
fn parse_gyear(dt: &mut DateVal, s: &[u8], cur: &mut usize) -> i32 {
    let mut c = *cur;
    let first = at(s, c);
    if !digit(first) && first != b'-' && first != b'+' {
        return -1;
    }
    let neg = first == b'-';
    if neg {
        c += 1;
    }
    let first_char = at(s, c);
    let mut count = 0;
    while digit(at(s, c)) {
        if dt.year >= i64::MAX / 10 {
            return -1;
        }
        dt.year = dt.year * 10 + i64::from(at(s, c) - b'0');
        c += 1;
        count += 1;
    }
    // At least four digits (CCYY); more than four can't start with 0.
    if count < 4 || (count > 4 && first_char == b'0') {
        return 1;
    }
    if dt.year == 0 {
        return 2;
    }
    if neg {
        dt.year = -dt.year + 1;
    }
    *cur = c;
    0
}

/// _exsltDateParseGMonth.
fn parse_gmonth(dt: &mut DateVal, s: &[u8], cur: &mut usize) -> i32 {
    let mut c = *cur;
    let ret = parse_2_digits(s, &mut c, |m| (1..=12).contains(&m), &mut dt.mon);
    if ret == 0 {
        *cur = c;
    }
    ret
}

/// _exsltDateParseGDay.
fn parse_gday(dt: &mut DateVal, s: &[u8], cur: &mut usize) -> i32 {
    let mut c = *cur;
    let ret = parse_2_digits(s, &mut c, |d| (1..=31).contains(&d), &mut dt.day);
    if ret == 0 {
        *cur = c;
    }
    ret
}

/// _exsltDateParseTime.
fn parse_time(dt: &mut DateVal, s: &[u8], cur: &mut usize) -> i32 {
    let mut c = *cur;
    let mut hour = 0;
    let ret = parse_2_digits(s, &mut c, |h| h <= 23, &mut hour);
    if ret != 0 {
        return ret;
    }
    if at(s, c) != b':' {
        return 1;
    }
    c += 1;
    // The ':' makes it an xs:time.
    dt.hour = hour;
    let ret = parse_2_digits(s, &mut c, |m| m <= 59, &mut dt.min);
    if ret != 0 {
        return ret;
    }
    if at(s, c) != b':' {
        return 1;
    }
    c += 1;
    // PARSE_FLOAT: two integer digits, then an optional fraction.
    let mut whole = 0;
    let mut ret = parse_2_digits(s, &mut c, |_| true, &mut whole);
    if ret == 0 {
        dt.sec = f64::from(whole);
        if at(s, c) == b'.' {
            let mut mult = 1.0;
            c += 1;
            if !digit(at(s, c)) {
                ret = 1;
            }
            while digit(at(s, c)) {
                mult /= 10.0;
                dt.sec += f64::from(at(s, c) - b'0') * mult;
                c += 1;
            }
        }
    }
    if ret != 0 {
        return ret;
    }
    if !valid_time(dt) {
        return 2;
    }
    *cur = c;
    0
}

/// _exsltDateParseTimeZone.
fn parse_timezone(dt: &mut DateVal, s: &[u8], cur: &mut usize) -> i32 {
    let mut c = *cur;
    match at(s, c) {
        0 => {
            dt.tz_flag = false;
            dt.tzo = 0;
        }
        b'Z' => {
            dt.tz_flag = true;
            dt.tzo = 0;
            c += 1;
        }
        sign @ (b'+' | b'-') => {
            c += 1;
            let mut tmp = 0;
            let ret = parse_2_digits(s, &mut c, |h| h <= 23, &mut tmp);
            if ret != 0 {
                return ret;
            }
            if at(s, c) != b':' {
                return 1;
            }
            c += 1;
            dt.tzo = tmp as i32 * 60;
            let ret = parse_2_digits(s, &mut c, |m| m <= 59, &mut tmp);
            if ret != 0 {
                return ret;
            }
            dt.tzo += tmp as i32;
            if sign == b'-' {
                dt.tzo = -dt.tzo;
            }
            if !(dt.tzo > -1440 && dt.tzo < 1440) {
                return 2;
            }
        }
        _ => return 1,
    }
    *cur = c;
    0
}

/// exsltDateParse: None if `s` isn't one of the date and time types.
fn parse(s: &str) -> Option<DateVal> {
    let s = s.as_bytes();
    let mut dt = DateVal::new(UNKNOWN);
    let mut cur = 0;

    // RETURN_TYPE_IF_VALID: a time zone (or the end) completes type `t`.
    let complete = |dt: &mut DateVal, cur: usize, t: u8| -> Option<Option<DateVal>> {
        if matches!(at(s, cur), 0 | b'Z' | b'+' | b'-') {
            let mut c = cur;
            if parse_timezone(dt, s, &mut c) == 0 {
                if at(s, c) != 0 {
                    return Some(None);
                }
                dt.ty = t;
                return Some(Some(*dt));
            }
        }
        None
    };

    if at(s, 0) == b'-' && at(s, 1) == b'-' {
        // xs:gMonthDay, xs:gMonth or xs:gDay
        cur += 2;
        if at(s, cur) == b'-' {
            cur += 1;
            if parse_gday(&mut dt, s, &mut cur) != 0 {
                return None;
            }
            return complete(&mut dt, cur, GDAY).flatten();
        }
        if parse_gmonth(&mut dt, s, &mut cur) != 0 {
            return None;
        }
        if at(s, cur) != b'-' {
            return None;
        }
        cur += 1;
        if at(s, cur) == b'-' {
            cur += 1;
            return complete(&mut dt, cur, GMONTH).flatten();
        }
        if parse_gday(&mut dt, s, &mut cur) != 0 {
            return None;
        }
        return complete(&mut dt, cur, GMONTHDAY).flatten();
    }

    // A right-truncated date or an xs:time: try the time first.
    if digit(at(s, cur))
        && parse_time(&mut dt, s, &mut cur) == 0
        && let Some(r) = complete(&mut dt, cur, TIME)
    {
        return r;
    }

    // Fall back on a date.
    cur = 0;
    if parse_gyear(&mut dt, s, &mut cur) != 0 {
        return None;
    }
    if let Some(r) = complete(&mut dt, cur, GYEAR) {
        return r;
    }
    if at(s, cur) != b'-' {
        return None;
    }
    cur += 1;
    if parse_gmonth(&mut dt, s, &mut cur) != 0 {
        return None;
    }
    if let Some(r) = complete(&mut dt, cur, GYEARMONTH) {
        return r;
    }
    if at(s, cur) != b'-' {
        return None;
    }
    cur += 1;
    if parse_gday(&mut dt, s, &mut cur) != 0 || !valid_date(&dt) {
        return None;
    }
    if let Some(r) = complete(&mut dt, cur, DATE) {
        return r;
    }
    if at(s, cur) != b'T' {
        return None;
    }
    cur += 1;
    if parse_time(&mut dt, s, &mut cur) != 0 {
        return None;
    }
    if parse_timezone(&mut dt, s, &mut cur) != 0 || at(s, cur) != 0 || !valid_datetime(&dt) {
        return None;
    }
    dt.ty = DATETIME;
    Some(dt)
}

/// strtol(s, NULL, 10) and whether errno stayed 0: leading white space, an
/// optional sign, then digits; no digits is 0, overflow is an error.
fn strtol(s: &str) -> Option<i64> {
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i64 = 0;
    for c in digits.bytes().take_while(u8::is_ascii_digit) {
        let d = i64::from(c - b'0');
        n = n.checked_mul(10)?;
        n = if neg {
            n.checked_sub(d)?
        } else {
            n.checked_add(d)?
        };
    }
    Some(n)
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// exsltDateCurrent: `SOURCE_DATE_EPOCH` if set and valid, else now; UTC
/// either way in oxslt (see the module comment).
fn current() -> DateVal {
    // gmtime_r fails when the year doesn't fit C's int; libexslt then
    // ignores SOURCE_DATE_EPOCH.
    let representable = |secs: i64| {
        let year = civil_from_days(secs.div_euclid(86_400)).0;
        i32::try_from(year - 1900).is_ok()
    };
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| strtol(&v))
        .filter(|&s| representable(s))
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
        });
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (year, mon, day) = civil_from_days(days);
    DateVal {
        ty: DATETIME,
        year,
        mon,
        day,
        hour: (rest / 3600) as u32,
        min: (rest / 60 % 60) as u32,
        sec: (rest % 60) as f64,
        tz_flag: false,
        tzo: 0,
    }
}

// ---------- formatting ----------

fn push_2_digits(out: &mut String, n: u32) {
    out.push(char::from(b'0' + (n / 10 % 10) as u8));
    out.push(char::from(b'0' + (n % 10) as u8));
}

/// FORMAT_GYEAR: at least four digits, '-' before years BC.
fn push_gyear(out: &mut String, yr: i64) {
    if yr <= 0 {
        out.push('-');
    }
    let year = if yr <= 0 { -yr + 1 } else { yr };
    out.push_str(&format!("{year:04}"));
}

fn push_date(out: &mut String, dt: &DateVal) {
    push_gyear(out, dt.year);
    out.push('-');
    push_2_digits(out, dt.mon);
    out.push('-');
    push_2_digits(out, dt.day);
}

/// FORMAT_TIME: seconds as XPath formats numbers, padded to two digits.
fn push_time(out: &mut String, dt: &DateVal) {
    push_2_digits(out, dt.hour);
    out.push(':');
    push_2_digits(out, dt.min);
    out.push(':');
    if dt.sec < 10.0 {
        out.push('0');
    }
    out.push_str(&xpath::number::to_string(dt.sec));
}

fn push_tz(out: &mut String, tzo: i32) {
    if tzo == 0 {
        out.push('Z');
    } else {
        let a = tzo.unsigned_abs();
        out.push(if tzo < 0 { '-' } else { '+' });
        push_2_digits(out, a / 60);
        out.push(':');
        push_2_digits(out, a % 60);
    }
}

fn format_datetime(dt: &DateVal) -> Option<String> {
    if !valid_datetime(dt) {
        return None;
    }
    let mut s = String::new();
    push_date(&mut s, dt);
    s.push('T');
    push_time(&mut s, dt);
    push_tz(&mut s, dt.tzo);
    Some(s)
}

fn format_date(dt: &DateVal) -> Option<String> {
    if !valid_datetime(dt) {
        return None;
    }
    let mut s = String::new();
    push_date(&mut s, dt);
    if dt.tz_flag || dt.tzo != 0 {
        push_tz(&mut s, dt.tzo);
    }
    Some(s)
}

fn format_time(dt: &DateVal) -> Option<String> {
    if !valid_time(dt) {
        return None;
    }
    let mut s = String::new();
    push_time(&mut s, dt);
    if dt.tz_flag || dt.tzo != 0 {
        push_tz(&mut s, dt.tzo);
    }
    Some(s)
}

/// _exsltDateDayInWeek: 0 for Sunday.
fn day_in_week(yday: i64, yr: i64) -> i64 {
    if yr <= 0 {
        let r = (yr % 7 - 2 + (yr / 4 - yr / 100 + yr / 400) + yday) % 7;
        if r < 0 { r + 7 } else { r }
    } else {
        (yr % 7 - 1 + ((yr - 1) / 4 - (yr - 1) / 100 + (yr - 1) / 400) + yday) % 7
    }
}

// ---------- the functions ----------

/// The argument parsed, or the current date and time without one; None if
/// it doesn't parse or isn't one of `types`.
fn value_of(arg: Option<&str>, types: &[u8]) -> Option<DateVal> {
    match arg {
        None => Some(current()),
        Some(s) => parse(s).filter(|dt| types.contains(&dt.ty)),
    }
}

const YEAR_TYPES: &[u8] = &[DATETIME, DATE, GYEARMONTH, GYEAR];
const MONTH_TYPES: &[u8] = &[DATETIME, DATE, GYEARMONTH, GMONTH, GMONTHDAY];
const DAY_TYPES: &[u8] = &[DATETIME, DATE, GMONTHDAY, GDAY];
const DATE_TYPES: &[u8] = &[DATETIME, DATE];
const TIME_TYPES: &[u8] = &[DATETIME, TIME];

fn month_in_year(arg: Option<&str>) -> f64 {
    value_of(arg, MONTH_TYPES).map_or(f64::NAN, |dt| f64::from(dt.mon))
}

fn day_in_week_of(arg: Option<&str>) -> f64 {
    value_of(arg, DATE_TYPES).map_or(f64::NAN, |dt| {
        (day_in_week(day_in_year(i64::from(dt.day), dt.mon, dt.year), dt.year) + 1) as f64
    })
}

/// A name by number (1-based), or "" when out of range or NaN.
fn name(names: &[&'static str], n: f64) -> &'static str {
    if !n.is_nan() && n >= 1.0 && n <= names.len() as f64 {
        names[n as usize - 1]
    } else {
        ""
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTH_ABBREVIATIONS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const DAY_ABBREVIATIONS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Calls `date:local`, one of `IMPLEMENTED`.
pub fn call(local: &str, args: &[Value], env: &Env) -> Result<Value, XPathError> {
    let max = if local == "date-time" { 0 } else { 1 };
    if args.len() > max {
        return Err(XPathError::runtime("Invalid number of arguments"));
    }
    let arg = args.first().map(|v| xpath::to_string(v, env));
    let arg = arg.as_deref();
    let string = |s: Option<String>| Value::String(s.unwrap_or_default());
    let number = Value::Number;
    Ok(match local {
        "date-time" => string(format_datetime(&current())),
        "date" => string(value_of(arg, DATE_TYPES).and_then(|dt| format_date(&dt))),
        "time" => string(value_of(arg, TIME_TYPES).and_then(|dt| format_time(&dt))),
        "year" => number(value_of(arg, YEAR_TYPES).map_or(f64::NAN, |dt| {
            // There's no year 0.
            (if dt.year <= 0 { dt.year - 1 } else { dt.year }) as f64
        })),
        "leap-year" => match value_of(arg, YEAR_TYPES) {
            Some(dt) => Value::Boolean(is_leap(dt.year)),
            None => Value::Number(f64::NAN),
        },
        "month-in-year" => number(month_in_year(arg)),
        "month-name" => Value::String(name(&MONTHS, month_in_year(arg)).to_string()),
        "month-abbreviation" => {
            Value::String(name(&MONTH_ABBREVIATIONS, month_in_year(arg)).to_string())
        }
        "week-in-year" => number(value_of(arg, DATE_TYPES).map_or(f64::NAN, |dt| {
            let mut diy = day_in_year(i64::from(dt.day), dt.mon, dt.year);
            // Monday first, then the ISO 8601 adjustment (3 is Thursday).
            let diw = (day_in_week(diy, dt.year) + 6) % 7;
            diy += 3 - diw;
            if diy < 1 {
                let mut year = dt.year - 1;
                if year == 0 {
                    year -= 1;
                }
                diy += day_in_year(31, 12, year);
            } else if diy > day_in_year(31, 12, dt.year) {
                diy -= day_in_year(31, 12, dt.year);
            }
            ((diy - 1) / 7 + 1) as f64
        })),
        "week-in-month" => number(value_of(arg, DATE_TYPES).map_or(f64::NAN, |dt| {
            let fdiy = day_in_year(1, dt.mon, dt.year);
            let fdiw = (day_in_week(fdiy, dt.year) + 6) % 7;
            ((i64::from(dt.day) + fdiw - 1) / 7 + 1) as f64
        })),
        "day-in-year" => number(value_of(arg, DATE_TYPES).map_or(f64::NAN, |dt| {
            day_in_year(i64::from(dt.day), dt.mon, dt.year) as f64
        })),
        "day-in-month" => number(value_of(arg, DAY_TYPES).map_or(f64::NAN, |dt| f64::from(dt.day))),
        "day-of-week-in-month" => {
            number(value_of(arg, DATE_TYPES).map_or(f64::NAN, |dt| f64::from((dt.day - 1) / 7 + 1)))
        }
        "day-in-week" => number(day_in_week_of(arg)),
        "day-name" => Value::String(name(&DAYS, day_in_week_of(arg)).to_string()),
        "day-abbreviation" => {
            Value::String(name(&DAY_ABBREVIATIONS, day_in_week_of(arg)).to_string())
        }
        "hour-in-day" => {
            number(value_of(arg, TIME_TYPES).map_or(f64::NAN, |dt| f64::from(dt.hour)))
        }
        "minute-in-hour" => {
            number(value_of(arg, TIME_TYPES).map_or(f64::NAN, |dt| f64::from(dt.min)))
        }
        "second-in-minute" => number(value_of(arg, TIME_TYPES).map_or(f64::NAN, |dt| dt.sec)),
        _ => unreachable!("only IMPLEMENTED functions are called"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_types_and_quirks() {
        let ty = |s: &str| parse(s).map(|d| d.ty);
        assert_eq!(ty("2024-02-29T13:05:09Z"), Some(DATETIME));
        assert_eq!(ty("2024-02-29"), Some(DATE));
        assert_eq!(ty("2023-02-29"), None);
        assert_eq!(ty("2024-02"), Some(GYEARMONTH));
        assert_eq!(ty("2024"), Some(GYEAR));
        assert_eq!(ty("13:05:09.5+01:30"), Some(TIME));
        assert_eq!(ty("--02-29"), Some(GMONTHDAY));
        assert_eq!(ty("--02--"), Some(GMONTH));
        assert_eq!(ty("---29"), Some(GDAY));
        assert_eq!(ty("0000"), None);
        assert_eq!(ty("024"), None);
        assert_eq!(ty("02024"), None);
        assert_eq!(ty("2024-02-29T24:00:00"), None);
        assert_eq!(ty("2024-02-29T13:05:09Zx"), None);
        assert_eq!(parse("-0001").unwrap().year, 0);
    }

    #[test]
    fn formatting() {
        let dt = parse("2024-02-29T03:05:09.25-05:30").unwrap();
        assert_eq!(
            format_datetime(&dt).unwrap(),
            "2024-02-29T03:05:09.25-05:30"
        );
        assert_eq!(format_date(&dt).unwrap(), "2024-02-29-05:30");
        assert_eq!(format_time(&dt).unwrap(), "03:05:09.25-05:30");
        let dt = parse("2024-02-29").unwrap();
        assert_eq!(format_date(&dt).unwrap(), "2024-02-29");
        assert_eq!(civil_from_days(19_675), (2023, 11, 14));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn strtol_like_c() {
        assert_eq!(strtol(" 1700000000x"), Some(1_700_000_000));
        assert_eq!(strtol("abc"), Some(0));
        assert_eq!(strtol("-5"), Some(-5));
        assert_eq!(strtol("99999999999999999999"), None);
    }
}
