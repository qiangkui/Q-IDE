//! VB6 财务函数：`SLN`/`SYD`/`DDB`/`FV`/`PV`/`Pmt`/`NPer`/`IPmt`/`PPmt`/`NPV`/`IRR`/`MIRR`/`Rate`。
//!
//! 闭式公式的求值顺序照本机 VB6 p-code 实测校准（`FV(0.05,10,-100)` = 1257.78925355488 等，
//! 见 `src/builtins_tests.rs`）；VB6 是 x87 长双精度，末位可能差 1 ulp，所以 `IRR`/`Rate`
//! 这类迭代解只保证约 1e-9 的相对误差。

use crate::interpreter::builtins::Ctx;
use crate::interpreter::value::Value;

/// 本模块实现的内置函数名（小写）。
pub const BUILTINS: &[&str] = &[
    "sln", "syd", "ddb", "fv", "pv", "pmt", "nper", "ipmt", "ppmt", "npv", "irr", "mirr", "rate",
];

/// 财务函数的失败：一个 VB 错误号 + 给 `Err.Description` 用的中文说明。
type Failure = (i64, String);

type FinResult = Result<f64, Failure>;

/// `错误号 5`：VB6 里参数不合法时财务函数统一报「无效的过程调用或参数」。
fn invalid(call: &str) -> Failure {
    (5, format!("`{call}` 的参数不合法"))
}

fn missing(index: usize) -> Failure {
    (449, format!("缺少第 {} 个参数", index + 1))
}

/// 财务函数的入口：命中本模块的函数返回 `Some`，否则返回 `None` 交回 `builtins::call`。
pub fn call(name: &str, args: &[Value], ctx: &mut Ctx) -> Result<Option<Value>, String> {
    if !BUILTINS.contains(&name) {
        return Ok(None);
    }
    match compute(name, args) {
        Ok(number) => Ok(Some(Value::Float(number))),
        Err((code, message)) => {
            ctx.failure = Some(code);
            Err(message)
        }
    }
}

fn compute(name: &str, args: &[Value]) -> FinResult {
    Ok(match name {
        "sln" => sln(value(args, 0)?, value(args, 1)?, value(args, 2)?)?,
        "syd" => syd(
            value(args, 0)?,
            value(args, 1)?,
            value(args, 2)?,
            value(args, 3)?,
        )?,
        "ddb" => ddb(
            value(args, 0)?,
            value(args, 1)?,
            value(args, 2)?,
            value(args, 3)?,
            default(args, 4, 2.0)?,
        )?,
        "fv" => fv(
            value(args, 0)?,
            value(args, 1)?,
            value(args, 2)?,
            default(args, 3, 0.0)?,
            type_flag(args, 4)?,
        ),
        "pv" => pv(
            value(args, 0)?,
            value(args, 1)?,
            value(args, 2)?,
            default(args, 3, 0.0)?,
            type_flag(args, 4)?,
        ),
        "pmt" => pmt(
            value(args, 0)?,
            value(args, 1)?,
            value(args, 2)?,
            default(args, 3, 0.0)?,
            type_flag(args, 4)?,
        )?,
        "nper" => nper(
            value(args, 0)?,
            value(args, 1)?,
            value(args, 2)?,
            default(args, 3, 0.0)?,
            type_flag(args, 4)?,
        )?,
        "ipmt" => ipmt(
            value(args, 0)?,
            value(args, 1)?,
            value(args, 2)?,
            value(args, 3)?,
            default(args, 4, 0.0)?,
            type_flag(args, 5)?,
        )?,
        "ppmt" => {
            let rate = value(args, 0)?;
            let per = value(args, 1)?;
            let periods = value(args, 2)?;
            let present = value(args, 3)?;
            let future = default(args, 4, 0.0)?;
            let due = type_flag(args, 5)?;
            let interest = ipmt(rate, per, periods, present, future, due)?;
            pmt(rate, periods, present, future, due)? - interest
        }
        "npv" => npv(value(args, 0)?, numbers(args, 1)?)?,
        "irr" => irr(numbers(args, 0)?, default(args, 1, 0.1)?)?,
        "mirr" => mirr(numbers(args, 0)?, value(args, 1)?, value(args, 2)?)?,
        "rate" => rate(
            value(args, 0)?,
            value(args, 1)?,
            value(args, 2)?,
            default(args, 3, 0.0)?,
            type_flag(args, 4)?,
            default(args, 5, 0.1)?,
        )?,
        _ => return Err(invalid(name)),
    })
}

/// 取一个必选数值实参。
fn value(args: &[Value], index: usize) -> Result<f64, Failure> {
    numeric(args, index)?.ok_or_else(|| missing(index))
}

/// 取一个可选数值实参：省略或 `Empty` 用默认值。
fn default(args: &[Value], index: usize, fallback: f64) -> Result<f64, Failure> {
    Ok(numeric(args, index)?.unwrap_or(fallback))
}

/// `type` 形参：VB6 接受 `True`（-1）、`1` 等任何非零值，效果都是「期初付款」。
fn type_flag(args: &[Value], index: usize) -> Result<f64, Failure> {
    Ok(if default(args, index, 0.0)? != 0.0 {
        1.0
    } else {
        0.0
    })
}

/// 数值实参的统一转换：省略 → `None`，`Empty` → 0，`Null` → 错误号 94，其它非数值 → 13。
fn numeric(args: &[Value], index: usize) -> Result<Option<f64>, Failure> {
    match args.get(index) {
        None => Ok(None),
        Some(Value::Empty) => Ok(Some(0.0)),
        Some(Value::Null) => Err((94, format!("第 {} 个参数是 `Null`", index + 1))),
        Some(argument) => match argument.as_float() {
            Ok(number) => Ok(Some(number)),
            Err(message) => Err((13, message)),
        },
    }
}

/// 取一个一维数值数组实参（`NPV`/`IRR`/`MIRR`）。
fn numbers(args: &[Value], index: usize) -> Result<Vec<f64>, Failure> {
    let argument = args.get(index).ok_or_else(|| missing(index))?;
    let array = argument.as_array().map_err(|message| (13, message))?;
    let array = array.borrow();
    if array.dims().len() != 1 {
        return Err((13, "财务函数只支持一维数组".into()));
    }
    let mut values = Vec::with_capacity(array.cells().len());
    for cell in array.cells() {
        let number = cell.borrow().as_float().map_err(|message| (13, message))?;
        values.push(number);
    }
    Ok(values)
}

fn sln(cost: f64, salvage: f64, life: f64) -> FinResult {
    if life == 0.0 {
        return Err(invalid("SLN"));
    }
    Ok((cost - salvage) / life)
}

fn syd(cost: f64, salvage: f64, life: f64, period: f64) -> FinResult {
    if life <= 0.0 || period < 1.0 || period > life {
        return Err(invalid("SYD"));
    }
    Ok((cost - salvage) * (life - period + 1.0) * 2.0 / (life * (life + 1.0)))
}

/// 双倍余额递减法：净值按 `(1 - factor/life)` 等比衰减，某期折旧不超过「期初净值 − 残值」。
///
/// VB6 支持小数期（`DDB(10000, 1000, 5, 4.5)` = 669.251522224642），所以用几何式而不是逐期循环；
/// 首期的钳制与后续各期不同（成本低于残值时首期直接给「成本 − 残值」，后续各期给 0）。
fn ddb(cost: f64, salvage: f64, life: f64, period: f64, factor: f64) -> FinResult {
    if life <= 0.0 || period < 1.0 || period > life || factor <= 0.0 {
        return Err(invalid("DDB"));
    }
    let rate = factor / life;
    if period == 1.0 {
        let depreciation = cost * rate;
        if depreciation > 0.0 && depreciation > cost - salvage {
            return Ok(cost - salvage);
        }
        return Ok(depreciation);
    }
    let opening = cost * (1.0 - rate).powf(period - 1.0);
    let mut depreciation = opening * rate;
    if depreciation > 0.0 && opening - depreciation < salvage {
        depreciation = (opening - salvage).max(0.0);
    }
    Ok(depreciation)
}

/// 终值：`FV(rate, nper, pmt, [pv], [type])`。
fn fv(rate: f64, nper: f64, pmt: f64, pv: f64, due: f64) -> f64 {
    if rate == 0.0 {
        return -(pv + pmt * nper);
    }
    let growth = (1.0 + rate).powf(nper);
    -(pv * growth + pmt * (1.0 + rate * due) * ((growth - 1.0) / rate))
}

fn pv(rate: f64, nper: f64, pmt: f64, fv: f64, due: f64) -> f64 {
    if rate == 0.0 {
        return -(fv + pmt * nper);
    }
    let growth = (1.0 + rate).powf(nper);
    -(fv + pmt * (1.0 + rate * due) * ((growth - 1.0) / rate)) / growth
}

fn pmt(rate: f64, nper: f64, pv: f64, fv: f64, due: f64) -> FinResult {
    if nper == 0.0 {
        return Err(invalid("Pmt"));
    }
    if rate == 0.0 {
        return Ok(-(fv + pv) / nper);
    }
    let growth = (1.0 + rate).powf(nper);
    Ok(-(fv + pv * growth) * rate / ((1.0 + rate * due) * (growth - 1.0)))
}

fn nper(rate: f64, payment: f64, present: f64, future: f64, due: f64) -> FinResult {
    if payment == 0.0 {
        return Err(invalid("NPer"));
    }
    if rate == 0.0 {
        return Ok(-(future + present) / payment);
    }
    let annuity = payment * (1.0 + rate * due);
    let ratio = (annuity - future * rate) / (present * rate + annuity);
    if ratio.is_nan() || ratio <= 0.0 {
        return Err(invalid("NPer"));
    }
    Ok(ratio.ln() / (1.0 + rate).ln())
}

/// 某期利息：期初付款时第 1 期利息为 0，其余各期按「上期付款后的余额」计息。
fn ipmt(rate: f64, per: f64, periods: f64, present: f64, future: f64, due: f64) -> FinResult {
    if per < 1.0 || per > periods {
        return Err(invalid("IPmt"));
    }
    if rate == 0.0 {
        return Ok(0.0);
    }
    let payment = pmt(rate, periods, present, future, due)?;
    let balance = if due == 0.0 {
        fv(rate, per - 1.0, payment, present, 0.0)
    } else {
        if per == 1.0 {
            return Ok(0.0);
        }
        fv(rate, per - 2.0, payment, present, 1.0) - payment
    };
    Ok(balance * rate)
}

/// 净现值：`values(i)` 视为第 `i + 1` 期（与 VB6 一致，不是第 0 期）。
fn npv(rate: f64, values: Vec<f64>) -> FinResult {
    if 1.0 + rate == 0.0 {
        return Err((11, "除数为零".into()));
    }
    let mut total = 0.0;
    for (index, value) in values.into_iter().enumerate() {
        total += value / (1.0 + rate).powf((index + 1) as f64);
    }
    Ok(total)
}

/// 割线法求根：VB6 的 `IRR`/`Rate` 也是迭代解，收敛判据是 `|Δx| < tol`。
fn solve(guess: f64, tolerance: f64, evaluate: impl Fn(f64) -> f64) -> Option<f64> {
    let mut low = guess;
    let mut low_value = evaluate(low);
    if low_value == 0.0 {
        return Some(guess);
    }
    let mut high = guess + 1e-5;
    let mut high_value = evaluate(high);
    for _ in 0..100 {
        if high_value == low_value {
            return None;
        }
        let next = high - high_value * (high - low) / (high_value - low_value);
        if !next.is_finite() {
            return None;
        }
        low = high;
        low_value = high_value;
        high = next;
        high_value = evaluate(high);
        if (high - low).abs() < tolerance {
            return Some(high);
        }
    }
    None
}

fn irr(values: Vec<f64>, guess: f64) -> FinResult {
    let (mut positive, mut negative) = (false, false);
    for value in &values {
        positive |= *value > 0.0;
        negative |= *value < 0.0;
    }
    if !positive || !negative {
        return Err(invalid("IRR"));
    }
    let cash_flow = |rate: f64| -> f64 {
        let mut total = 0.0;
        for (index, value) in values.iter().enumerate() {
            total += value / (1.0 + rate).powf((index + 1) as f64);
        }
        total
    };
    solve(guess, 1e-7, cash_flow).ok_or_else(|| invalid("IRR"))
}

/// 修正内部收益率：正现金流按再投资率滚到期末、负现金流按融资率折到起初，再开 `n - 1` 次方。
fn mirr(values: Vec<f64>, finance_rate: f64, reinvest_rate: f64) -> FinResult {
    let count = values.len();
    if count < 2 {
        return Err(invalid("MIRR"));
    }
    let mut positives = 0.0;
    let mut negatives = 0.0;
    for (index, value) in values.iter().enumerate() {
        if *value > 0.0 {
            positives += value / (1.0 + reinvest_rate).powf((index + 1) as f64);
        } else if *value < 0.0 {
            negatives += value / (1.0 + finance_rate).powf((index + 1) as f64);
        }
    }
    if negatives == 0.0 {
        // VB6 实测：全是正现金流时报「除数为零」（错误号 11）。
        return Err((11, "除数为零".into()));
    }
    let ratio =
        -positives * (1.0 + reinvest_rate).powf(count as f64) / (negatives * (1.0 + finance_rate));
    if ratio < 0.0 {
        return Err(invalid("MIRR"));
    }
    Ok(ratio.powf(1.0 / (count as f64 - 1.0)) - 1.0)
}

/// 各期利率：解 `FV(rate, nper, pmt, pv, type) = fv`。
///
/// 零利率恰好是解时 VB6 报错误号 5（实测 `Rate(10,-100,1000)` 就报错），这里保持同样的行为。
fn rate(periods: f64, payment: f64, present: f64, future: f64, due: f64, guess: f64) -> FinResult {
    if periods <= 0.0 {
        return Err(invalid("Rate"));
    }
    let balance = |rate: f64| fv(rate, periods, payment, present, due) - future;
    if balance(0.0) == 0.0 {
        return Err(invalid("Rate"));
    }
    // VB6 的 `Rate` 收敛得比 `IRR` 紧（`Rate(360,-1000,200000)` 与真根只差 1 ulp）。
    let found = solve(guess, 1e-9, balance).ok_or_else(|| invalid("Rate"))?;
    if found <= -1.0 {
        return Err(invalid("Rate"));
    }
    Ok(found)
}
