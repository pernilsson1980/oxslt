//! EXSLT's durations, ported from libexslt's date.c: parsing and
//! formatting them, adding them to dates and to each other, differences,
//! and date:seconds, date:duration and date:sum.

use super::*;

// ---------- durations ----------

/// exsltDateDurVal: months (years too), days, and seconds within a day
/// (0 <= sec < 86400, the sign carried by the day count).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DurVal {
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
pub(super) fn parse_duration(s: &str) -> Option<DurVal> {
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
pub(super) fn format_duration(dur: &DurVal) -> String {
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
pub(super) fn format_any(dt: &DateVal) -> Option<String> {
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
pub(super) fn date_add(dt: &DateVal, dur: &DurVal) -> Option<DateVal> {
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
pub(super) fn date_difference(x: &DateVal, y: &DateVal, flag: bool) -> Option<DurVal> {
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
pub(super) fn add_durations(x: &DurVal, y: &DurVal) -> Option<DurVal> {
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
pub(super) fn seconds(arg: Option<&str>) -> f64 {
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
pub(super) fn duration(arg: Option<&str>) -> Option<String> {
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
pub(super) fn sum(values: &[String]) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let mut total = DurVal::default();
    for v in values {
        total = add_durations(&total, &parse_duration(v)?)?;
    }
    Some(format_duration(&total))
}
