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
    "add",
    "add-duration",
    "difference",
    "duration",
    "seconds",
    "sum",
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

// ---------- durations ----------

/// exsltDateDurVal: months (years too), days, and seconds within a day
/// (0 <= sec < 86400, the sign carried by the day count).
#[derive(Clone, Copy, Debug, Default)]
struct DurVal {
    mon: i64,
    day: i64,
    sec: f64,
}

const SECS_PER_MIN: i64 = 60;
const SECS_PER_HOUR: i64 = 3600;
const SECS_PER_DAY: i64 = 86_400;
const MINS_PER_DAY: i64 = 1440;
const HOURS_PER_DAY: i64 = 24;
const DAYS_PER_EPOCH: i64 = 400 * 365 + 100 - 4 + 1;
const YEARS_PER_EPOCH: i64 = 400;
const YEAR_MAX: i64 = i64::MAX;
const YEAR_MIN: i64 = -i64::MAX + 1;

fn max_day_in_month(yr: i64, mon: u32) -> i64 {
    let table = if is_leap(yr) {
        &DAYS_IN_MONTH_LEAP
    } else {
        &DAYS_IN_MONTH
    };
    table[(mon as usize).clamp(1, 12) - 1]
}

/// exsltDateParseDuration: `-?PnYnMnDTnHnMn.nS`.
fn parse_duration(s: &str) -> Option<DurVal> {
    let s = s.as_bytes();
    let mut cur = 0;
    let mut neg = false;
    if at(s, cur) == b'-' {
        neg = true;
        cur += 1;
    }
    if at(s, cur) != b'P' {
        return None;
    }
    cur += 1;
    if at(s, cur) == 0 {
        return None;
    }
    let mut dur = DurVal::default();
    let mut seq = 0usize;
    let mut secs: i64 = 0;
    let mut sec_frac = 0.0;
    const DESIG: [u8; 6] = *b"YMDHMS";
    while at(s, cur) != 0 {
        let mut num: i64 = 0;
        let mut has_digits = false;
        let mut has_frac = false;
        if seq >= DESIG.len() {
            return None;
        }
        if at(s, cur) == b'T' {
            if seq > 3 {
                return None;
            }
            cur += 1;
            seq = 3;
        } else if seq == 3 {
            return None;
        }
        while digit(at(s, cur)) {
            let d = i64::from(at(s, cur) - b'0');
            num = num.checked_mul(10)?.checked_add(d)?;
            has_digits = true;
            cur += 1;
        }
        if at(s, cur) == b'.' {
            let mut mult = 1.0;
            cur += 1;
            has_frac = true;
            while digit(at(s, cur)) {
                mult /= 10.0;
                sec_frac += f64::from(at(s, cur) - b'0') * mult;
                has_digits = true;
                cur += 1;
            }
        }
        while at(s, cur) != DESIG[seq] {
            seq += 1;
            if seq == 3 || seq == DESIG.len() {
                return None;
            }
        }
        cur += 1;
        if !has_digits || (has_frac && seq != 5) {
            return None;
        }
        match seq {
            0 => dur.mon = num.checked_mul(12)?,
            1 => dur.mon = dur.mon.checked_add(num)?,
            2 => dur.day = num,
            3 => {
                dur.day = dur.day.checked_add(num / HOURS_PER_DAY)?;
                secs = (num % HOURS_PER_DAY) * SECS_PER_HOUR;
            }
            4 => {
                dur.day = dur.day.checked_add(num / MINS_PER_DAY)?;
                secs += (num % MINS_PER_DAY) * SECS_PER_MIN;
            }
            _ => {
                dur.day = dur.day.checked_add(num / SECS_PER_DAY)?;
                secs += num % SECS_PER_DAY;
            }
        }
        seq += 1;
    }
    dur.day = dur.day.checked_add(secs / SECS_PER_DAY)?;
    dur.sec = (secs % SECS_PER_DAY) as f64 + sec_frac;
    if neg {
        dur.mon = -dur.mon;
        dur.day = -dur.day;
        if dur.sec != 0.0 {
            dur.sec = SECS_PER_DAY as f64 - dur.sec;
            dur.day -= 1;
        }
    }
    Some(dur)
}

/// FORMAT_ITEM: whole `limit`s of `num`, then the designator.
fn format_item(num: &mut f64, out: &mut String, limit: f64, item: char) {
    if *num >= limit {
        let comp = (*num / limit).floor();
        out.push_str(&xpath::number::to_string(comp));
        out.push(item);
        *num -= comp * limit;
    }
}

/// exsltDateFormatDuration.
fn format_duration(dur: &DurVal) -> String {
    if dur.sec == 0.0 && dur.day == 0 && dur.mon == 0 {
        return "P0D".into();
    }
    let mut secs = dur.sec;
    let mut days = dur.day as f64;
    let mut years = (dur.mon / 12) as f64;
    let mut months = (dur.mon % 12) as f64;
    let mut neg = false;
    if days < 0.0 {
        if secs != 0.0 {
            secs = SECS_PER_DAY as f64 - secs;
            days += 1.0;
        }
        days = -days;
        neg = true;
    }
    if years < 0.0 {
        years = -years;
        neg = true;
    }
    if months < 0.0 {
        months = -months;
        neg = true;
    }
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.push('P');
    if years != 0.0 {
        format_item(&mut years, &mut out, 1.0, 'Y');
    }
    if months != 0.0 {
        format_item(&mut months, &mut out, 1.0, 'M');
    }
    format_item(&mut days, &mut out, 1.0, 'D');
    if secs > 0.0 {
        out.push('T');
    }
    format_item(&mut secs, &mut out, SECS_PER_HOUR as f64, 'H');
    format_item(&mut secs, &mut out, SECS_PER_MIN as f64, 'M');
    if secs > 0.0 {
        out.push_str(&xpath::number::to_string(secs));
        out.push('S');
    }
    out
}

/// exsltDateFormat: any type a sum or difference can give.
fn format_any(dt: &DateVal) -> Option<String> {
    match dt.ty {
        DATETIME => format_datetime(dt),
        DATE => format_date(dt),
        TIME => format_time(dt),
        t if t & GYEAR != 0 => {
            let mut s = String::new();
            push_gyear(&mut s, dt.year);
            if t == GYEARMONTH {
                s.push('-');
                push_2_digits(&mut s, dt.mon);
            }
            if dt.tz_flag || dt.tzo != 0 {
                push_tz(&mut s, dt.tzo);
            }
            Some(s)
        }
        _ => None,
    }
}

/// _exsltDateCastYMToDays: days before the date's month.
fn cast_ym_to_days(dt: &DateVal) -> i64 {
    let y = dt.year;
    if y <= 0 {
        (y - 1) * 365 + (y / 4 - y / 100 + y / 400) + day_in_year(0, dt.mon, y) - 1
    } else {
        (y - 1) * 365 + ((y - 1) / 4 - (y - 1) / 100 + (y - 1) / 400) + day_in_year(0, dt.mon, y)
    }
}

/// TIME_TO_NUMBER.
fn time_to_number(dt: &DateVal) -> f64 {
    f64::from(dt.hour * 3600 + dt.min * 60) + dt.sec
}

/// _exsltDateTruncateDate.
fn truncate(dt: &mut DateVal, ty: u8) {
    if ty & TIME != TIME {
        dt.hour = 0;
        dt.min = 0;
        dt.sec = 0.0;
    }
    if ty & GDAY != GDAY {
        dt.day = 1;
    }
    if ty & GMONTH != GMONTH {
        dt.mon = 1;
    }
    if ty & GYEAR != GYEAR {
        dt.year = 0;
    }
    dt.ty = ty;
}

/// _exsltDateAdd: a date plus a duration, keeping the date's type unless
/// the result needs a finer one.
fn date_add(dt: &DateVal, dur: &DurVal) -> Option<DateVal> {
    let mut ret = DateVal::new(dt.ty);
    // month
    let mut temp = i64::from(dt.mon) + dur.mon % 12;
    let mut carry = dur.mon / 12;
    if temp < 1 {
        temp += 12;
        carry -= 1;
    } else if temp > 12 {
        temp -= 12;
        carry += 1;
    }
    ret.mon = temp as u32;
    // year, with whole 400-year epochs of the day count
    carry = carry.wrapping_add((dur.day / DAYS_PER_EPOCH) * YEARS_PER_EPOCH);
    if (carry > 0 && dt.year > YEAR_MAX - carry) || (carry < 0 && dt.year < YEAR_MIN - carry) {
        return None;
    }
    ret.year = dt.year + carry;
    ret.tzo = dt.tzo;
    ret.tz_flag = dt.tz_flag;
    // seconds
    let sum = dt.sec + dur.sec;
    ret.sec = sum % 60.0;
    carry = (sum / 60.0) as i64;
    // minutes
    temp = i64::from(dt.min) + carry % 60;
    carry /= 60;
    if temp >= 60 {
        temp -= 60;
        carry += 1;
    }
    ret.min = temp as u32;
    // hours
    temp = i64::from(dt.hour) + carry % 24;
    carry /= 24;
    if temp >= 24 {
        temp -= 24;
        carry += 1;
    }
    ret.hour = temp as u32;
    // days
    let max = max_day_in_month(ret.year, ret.mon);
    temp = if i64::from(dt.day) > max {
        max
    } else if dt.day < 1 {
        1
    } else {
        i64::from(dt.day)
    };
    temp += dur.day % DAYS_PER_EPOCH + carry;
    loop {
        if temp < 1 {
            if ret.mon > 1 {
                ret.mon -= 1;
            } else {
                if ret.year == YEAR_MIN {
                    return None;
                }
                ret.mon = 12;
                ret.year -= 1;
            }
            temp += max_day_in_month(ret.year, ret.mon);
        } else if temp > max_day_in_month(ret.year, ret.mon) {
            temp -= max_day_in_month(ret.year, ret.mon);
            if ret.mon < 12 {
                ret.mon += 1;
            } else {
                if ret.year == YEAR_MAX {
                    return None;
                }
                ret.mon = 1;
                ret.year += 1;
            }
        } else {
            break;
        }
    }
    ret.day = temp as u32;
    if ret.ty != DATETIME {
        if ret.hour != 0 || ret.min != 0 || ret.sec != 0.0 {
            ret.ty = DATETIME;
        } else if ret.ty != DATE {
            if ret.day != 1 {
                ret.ty = DATE;
            } else if ret.ty != GYEARMONTH && ret.mon != 1 {
                ret.ty = GYEARMONTH;
            }
        }
    }
    Some(ret)
}

/// _exsltDateDifference: `y - x`. Different types are truncated to the
/// coarser one; years and months differ in months unless `flag`.
fn date_difference(x: &DateVal, y: &DateVal, flag: bool) -> Option<DurVal> {
    let ok = |t: u8| (GYEAR..=DATETIME).contains(&t);
    if !ok(x.ty) || !ok(y.ty) {
        return None;
    }
    let (mut x, mut y) = (*x, *y);
    if x.ty != y.ty {
        if x.ty < y.ty {
            truncate(&mut y, x.ty);
        } else {
            truncate(&mut x, y.ty);
        }
    }
    let mut ret = DurVal::default();
    if (x.ty == GYEAR || x.ty == GYEARMONTH) && !flag {
        let limit = i64::MAX / 24;
        if x.year >= limit || x.year <= i64::MIN / 24 || y.year >= limit || y.year <= i64::MIN / 24
        {
            return None;
        }
        ret.mon = (y.year - x.year) * 12 + (i64::from(y.mon) - i64::from(x.mon));
    } else {
        let limit = i64::MAX / 731;
        if x.year > limit || x.year < i64::MIN / 731 || y.year > limit || y.year < i64::MIN / 731 {
            return None;
        }
        ret.sec = time_to_number(&y) - time_to_number(&x);
        ret.sec += f64::from((x.tzo - y.tzo) * 60);
        let carry = (ret.sec / SECS_PER_DAY as f64).floor() as i64;
        ret.sec -= carry as f64 * SECS_PER_DAY as f64;
        ret.day = cast_ym_to_days(&y) - cast_ym_to_days(&x);
        ret.day += i64::from(y.day) - i64::from(x.day);
        ret.day += carry;
    }
    Some(ret)
}

/// _exsltDateAddDurCalc: false on overflow or a result with mixed signs.
fn add_durations(x: &DurVal, y: &DurVal) -> Option<DurVal> {
    let mut ret = DurVal {
        mon: x.mon.checked_add(y.mon)?,
        day: x.day.checked_add(y.day)?,
        sec: x.sec + y.sec,
    };
    if ret.sec >= SECS_PER_DAY as f64 {
        ret.sec -= SECS_PER_DAY as f64;
        ret.day = ret.day.checked_add(1)?;
    }
    if ret.day >= 0 {
        if (ret.day > 0 || ret.sec > 0.0) && ret.mon < 0 {
            return None;
        }
    } else if ret.mon > 0 {
        return None;
    }
    Some(ret)
}

/// exsltDateSeconds: seconds since 1970-01-01T00:00:00Z of a date (or the
/// current time), or the seconds of a duration without months.
fn seconds(arg: Option<&str>) -> f64 {
    let (dt, dur) = match arg {
        None => (Some(current()), None),
        Some(s) => match parse(s) {
            Some(dt) => (Some(dt), None),
            None => (None, parse_duration(s)),
        },
    };
    if let Some(dt) = dt {
        if dt.ty >= GYEAR {
            let mut y = DateVal::new(DATETIME);
            y.year = 1970;
            y.tz_flag = true;
            if let Some(diff) = date_difference(&y, &dt, true) {
                return diff.day as f64 * SECS_PER_DAY as f64 + diff.sec;
            }
        }
        return f64::NAN;
    }
    match dur {
        Some(d) if d.mon == 0 => d.day as f64 * SECS_PER_DAY as f64 + d.sec,
        _ => f64::NAN,
    }
}

/// exsltDateDuration: a number of seconds (or the current time's) as a
/// duration in days and seconds.
fn duration(arg: Option<&str>) -> Option<String> {
    let secs = match arg {
        None => seconds(None),
        Some(s) => xpath::number::from_string(s),
    };
    if secs.is_nan() {
        return None;
    }
    let days = (secs / SECS_PER_DAY as f64).floor();
    if days <= i64::MIN as f64 || days >= i64::MAX as f64 {
        return None;
    }
    Some(format_duration(&DurVal {
        mon: 0,
        day: days as i64,
        sec: secs - days * SECS_PER_DAY as f64,
    }))
}

/// date:sum: the durations of a node-set's string values added up; "" if
/// one isn't a duration or the sum fails.
fn sum(values: &[String]) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let mut total = DurVal::default();
    for v in values {
        total = add_durations(&total, &parse_duration(v)?)?;
    }
    Some(format_duration(&total))
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
    let (min, max) = match local {
        "date-time" => (0, 0),
        "add" | "add-duration" | "difference" => (2, 2),
        "sum" => (1, 1),
        _ => (0, 1),
    };
    if args.len() < min || args.len() > max {
        return Err(XPathError::runtime("Invalid number of arguments"));
    }
    let string_of = |i: usize| xpath::to_string(&args[i], env);
    match local {
        "add" => {
            let (x, y) = (string_of(0), string_of(1));
            let r = parse(&x)
                .filter(|dt| (GYEAR..=DATETIME).contains(&dt.ty))
                .and_then(|dt| date_add(&dt, &parse_duration(&y)?))
                .and_then(|dt| format_any(&dt));
            return Ok(Value::String(r.unwrap_or_default()));
        }
        "add-duration" => {
            let (x, y) = (string_of(0), string_of(1));
            let r = parse_duration(&x)
                .and_then(|a| add_durations(&a, &parse_duration(&y)?))
                .map(|d| format_duration(&d));
            return Ok(Value::String(r.unwrap_or_default()));
        }
        "difference" => {
            let (x, y) = (string_of(0), string_of(1));
            let r = parse(&x)
                .and_then(|a| date_difference(&a, &parse(&y)?, false))
                .map(|d| format_duration(&d));
            return Ok(Value::String(r.unwrap_or_default()));
        }
        "sum" => {
            // xmlXPathPopNodeSet: a node-set or a fragment.
            let nodes = args[0]
                .nodes_or_tree()
                .ok_or_else(|| XPathError::type_error("Invalid type"))?;
            let values: Vec<String> = nodes.iter().map(|&n| env.string_value(n)).collect();
            return Ok(Value::String(sum(&values).unwrap_or_default()));
        }
        _ => {}
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
        "seconds" => number(seconds(arg)),
        "duration" => string(duration(arg)),
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
    fn durations() {
        let round = |s: &str| parse_duration(s).map(|d| format_duration(&d));
        assert_eq!(
            round("P1Y2M3DT4H5M6.5S").as_deref(),
            Some("P1Y2M3DT4H5M6.5S")
        );
        assert_eq!(round("PT36H").as_deref(), Some("P1DT12H"));
        assert_eq!(round("-P1DT1S").as_deref(), Some("-P1DT1S"));
        assert_eq!(round("P0D").as_deref(), Some("P0D"));
        assert_eq!(round("P"), None);
        assert_eq!(round("P1H"), None);
        assert_eq!(round("P1.5D"), None);
        assert_eq!(round("P99999999999999999999Y"), None);
        let add = |d: &str, u: &str| {
            date_add(&parse(d).unwrap(), &parse_duration(u).unwrap()).and_then(|r| format_any(&r))
        };
        assert_eq!(add("2000-01-31", "P1M").as_deref(), Some("2000-02-29"));
        assert_eq!(add("2004-02", "P1D").as_deref(), Some("2004-02-02"));
        assert_eq!(
            add("2000-01-01T00:00:00Z", "-PT0.5S").as_deref(),
            Some("1999-12-31T23:59:59.5Z")
        );
        assert_eq!(seconds(Some("1970-01-02")), 86_400.0);
        assert!(seconds(Some("P1M")).is_nan());
        assert_eq!(duration(Some("-0.5")).as_deref(), Some("-PT0.5S"));
        assert_eq!(sum(&["P1D".into(), "-P1M".into()]), None);
    }

    #[test]
    fn strtol_like_c() {
        assert_eq!(strtol(" 1700000000x"), Some(1_700_000_000));
        assert_eq!(strtol("abc"), Some(0));
        assert_eq!(strtol("-5"), Some(-5));
        assert_eq!(strtol("99999999999999999999"), None);
    }
}
