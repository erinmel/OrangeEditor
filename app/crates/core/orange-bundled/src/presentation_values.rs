//! Deterministic color and time operations. The wall clock is a host effect.
use std::sync::Arc;

use orange_kernel::{BindingRequest, KernelBinding, KernelFault, PureKernel, ResolveError};
use orange_model::Value;
use orange_model::catalog::{NodeDefinition, Ports, ProviderDescriptor, ProviderKey};
use orange_model::conversion::{TIME_UNIT_NAMES, TIME_UNITS};
use orange_model::document::{
    DefinitionRef, NodeKind, OutputSource, PortDescriptor, literal::Literal,
};
use orange_model::id::{PortKey, TypeExpr};
use orange_model::value::{RGBA_TYPE, RecordRepr, parse_rgba_hex, rgba_hex};
use serde_json::json;

use crate::{CatalogError, KernelError, MathPackage};

const STRING: &str = "core/string@1";
const I64: &str = "core/i64@1";
const F64: &str = "core/f64@1";
const CHANNEL: &str = "core/u32@1";
const STAMP: &str = "core/timestamp@1";
const DURATION: &str = "core/duration@1";
const RGBA: &[(&str, &str)] = &[
    ("red", CHANNEL),
    ("green", CHANNEL),
    ("blue", CHANNEL),
    ("alpha", CHANNEL),
];
const STAMP_FORMATS: &[&str] = &[
    "rfc3339",
    "unix-seconds",
    "unix-milliseconds",
    "unix-microseconds",
    "unix-nanoseconds",
];
const STAMP_FRACTION_DIGITS: [u32; 5] = [0, 9, 6, 3, 0];
const DURATION_UNITS: &[&str] = &["milliseconds", "seconds", "minutes", "hours", "nanoseconds"];

#[derive(Debug, Clone, Copy)]
enum Op {
    Color,
    MakeRgba,
    SplitRgba,
    ParseColor,
    FormatColor,
    MakeDuration,
    DurationInUnits,
    ConvertTimeUnits,
    AddDuration,
    SubtractDuration,
    TimeBetween,
    ParseTimestamp,
    FormatTimestamp,
}

impl Op {
    const ALL: &[Self] = &[
        Self::Color,
        Self::MakeRgba,
        Self::SplitRgba,
        Self::ParseColor,
        Self::FormatColor,
        Self::MakeDuration,
        Self::DurationInUnits,
        Self::ConvertTimeUnits,
        Self::AddDuration,
        Self::SubtractDuration,
        Self::TimeBetween,
        Self::ParseTimestamp,
        Self::FormatTimestamp,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::Color => "org.orange.color/color",
            Self::MakeRgba => "org.orange.color/make-rgba",
            Self::SplitRgba => "org.orange.color/split-rgba",
            Self::ParseColor => "org.orange.color/parse-hex",
            Self::FormatColor => "org.orange.color/format-hex",
            Self::MakeDuration => "org.orange.time/make-duration",
            Self::DurationInUnits => "org.orange.time/duration-in-units",
            Self::ConvertTimeUnits => "org.orange.time/convert-time-units",
            Self::AddDuration => "org.orange.time/add-duration",
            Self::SubtractDuration => "org.orange.time/subtract-duration",
            Self::TimeBetween => "org.orange.time/time-between",
            Self::ParseTimestamp => "org.orange.time/parse-timestamp",
            Self::FormatTimestamp => "org.orange.time/format-timestamp",
        }
    }

    fn inputs(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Color | Self::SplitRgba | Self::FormatColor => &[("color", RGBA_TYPE)],
            Self::MakeRgba => RGBA,
            Self::ParseColor | Self::ParseTimestamp => &[("text", STRING)],
            Self::MakeDuration => &[("amount", I64)],
            Self::DurationInUnits => &[("duration", DURATION)],
            Self::ConvertTimeUnits => &[("amount", F64)],
            Self::AddDuration | Self::SubtractDuration => {
                &[("timestamp", STAMP), ("duration", DURATION)]
            }
            Self::TimeBetween => &[("start", STAMP), ("end", STAMP)],
            Self::FormatTimestamp => &[("timestamp", STAMP), ("utc_offset_minutes", I64)],
        }
    }

    fn outputs(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Color | Self::MakeRgba | Self::ParseColor => &[("result", RGBA_TYPE)],
            Self::SplitRgba => RGBA,
            Self::FormatColor | Self::FormatTimestamp => &[("result", STRING)],
            Self::MakeDuration | Self::TimeBetween => &[("result", DURATION)],
            Self::DurationInUnits => &[("result", I64)],
            Self::ConvertTimeUnits => &[("result", F64)],
            Self::AddDuration | Self::SubtractDuration | Self::ParseTimestamp => {
                &[("result", STAMP)]
            }
        }
    }

    fn choices(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Self::ParseTimestamp | Self::FormatTimestamp => Some(("format", STAMP_FORMATS)),
            Self::MakeDuration | Self::DurationInUnits => Some(("unit", DURATION_UNITS)),
            Self::ConvertTimeUnits => Some(("from_unit", &TIME_UNIT_NAMES)),
            _ => None,
        }
    }

    fn provider(self, package: &MathPackage) -> &ProviderKey {
        if self.id().starts_with("org.orange.color/") {
            &package.color
        } else {
            &package.time
        }
    }

    fn definition(self) -> Result<NodeDefinition, CatalogError> {
        let mut ports = Vec::new();
        for &(name, ty) in self.inputs() {
            let ty: TypeExpr = ty.parse()?;
            let mut port = PortDescriptor::data_in(name, ty.clone());
            let default = match (self, name) {
                (Self::Color, "color") => {
                    Some(json!({"red":76,"green":146,"blue":245,"alpha":255}))
                }
                (Self::MakeRgba, "alpha") => Some(json!(255)),
                (Self::MakeRgba, _) => Some(json!(0)),
                (Self::FormatTimestamp, "utc_offset_minutes") => Some(json!("0")),
                _ => None,
            };
            port.default = default.map(|value| Literal::value(ty, value));
            ports.push(port);
        }
        for &(name, ty) in self.outputs() {
            ports.push(PortDescriptor::data_out(
                name,
                ty.parse()?,
                OutputSource::Computed,
            ));
        }
        let mut properties = serde_json::Map::new();
        if let Some((key, choices)) = self.choices() {
            properties.insert(key.into(), json!({"enum":choices,"default":choices[0]}));
        }
        if matches!(self, Self::ConvertTimeUnits) {
            properties.insert(
                "to_unit".into(),
                json!({"enum":TIME_UNIT_NAMES,"default":"milliseconds"}),
            );
        }
        Ok(NodeDefinition {
            definition: DefinitionRef {
                id: self.id().parse()?,
                version: 1,
            },
            config_version: 1,
            kind: NodeKind::Pure,
            volatility: None,
            determinism: orange_model::catalog::Determinism::Deterministic,
            suspends: None,
            type_params: vec![],
            ports: Ports::Declared(ports),
            references: vec![],
            specializations: None,
            config_schema: Some(
                json!({"$schema":"orange.config-schema/v1","type":"object","properties":properties,"additionalProperties":false}),
            ),
        })
    }
}

pub(crate) fn register(
    package: &MathPackage,
    mut builder: orange_model::catalog::RegistryBuilder,
) -> Result<orange_model::catalog::RegistryBuilder, CatalogError> {
    use orange_model::type_term::TypeTerm;
    use orange_model::value::literal_codec::{FieldSchema, RecordSchema};
    let channel = TypeTerm::parse(&CHANNEL.parse()?)?;
    builder = builder.record_type(
        &package.color.package,
        &RGBA_TYPE.parse()?,
        RecordSchema {
            fields: RGBA
                .iter()
                .map(|(field, _)| FieldSchema {
                    field_id: (*field).to_owned(),
                    ty: channel.clone(),
                    optional: false,
                })
                .collect(),
        },
    )?;
    for key in [&package.color, &package.time] {
        let mut provider = ProviderDescriptor {
            key: key.clone(),
            implements: Default::default(),
            specializations: Default::default(),
        };
        for &op in Op::ALL.iter().filter(|op| op.provider(package) == key) {
            let definition = op.definition()?;
            provider
                .implements
                .insert((definition.definition.clone(), 1));
            provider
                .specializations
                .insert(definition.definition.clone(), vec![Default::default()]);
            builder = builder.definition(definition);
        }
        builder = builder.provider(provider);
    }
    Ok(builder)
}

pub(crate) fn bind(
    package: &MathPackage,
    request: &BindingRequest<'_>,
) -> Option<Result<KernelBinding, ResolveError>> {
    let op = Op::ALL
        .iter()
        .copied()
        .find(|op| op.id() == request.definition().id.as_str())?;
    Some(bind_op(op, package, request))
}

fn bind_op(
    op: Op,
    package: &MathPackage,
    request: &BindingRequest<'_>,
) -> Result<KernelBinding, ResolveError> {
    let provider = op.provider(package);
    if request.provider() != Some(provider)
        || request.definition().version != 1
        || request.config_version() != 1
        || !request
            .snapshot()
            .provides(request.definition(), 1, provider)
    {
        return Err(ResolveError::Unavailable);
    }
    let choice = op.choices().map_or(Ok(0), |(name, choices)| {
        request.config().get(name).map_or(Ok(0), |value| {
            choices
                .iter()
                .position(|choice| value.as_str() == Some(choice))
                .ok_or(ResolveError::Contract)
        })
    })?;
    let unit_scale = |name| {
        TIME_UNITS
            .iter()
            .find_map(|&(unit, scale)| (unit == name).then_some(scale))
            .ok_or(ResolveError::Contract)
    };
    let scale = if matches!(op, Op::ConvertTimeUnits) {
        let to_unit = request
            .config()
            .get("to_unit")
            .map_or(Ok("milliseconds"), |value| {
                value.as_str().ok_or(ResolveError::Contract)
            })?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "time unit scales are exact in binary64"
        )]
        let scale = unit_scale(TIME_UNIT_NAMES[choice])? as f64 / unit_scale(to_unit)? as f64;
        scale
    } else {
        1.0
    };
    let ports = |fields: &[(&str, &str)]| -> Result<Box<[PortKey]>, ResolveError> {
        fields
            .iter()
            .map(|(name, _)| name.parse().map_err(|_| ResolveError::Contract))
            .collect()
    };
    Ok(KernelBinding {
        inputs: ports(op.inputs())?,
        outputs: ports(op.outputs())?,
        kernel: Arc::new(Kernel {
            op,
            choice,
            scale,
            rgba: RGBA_TYPE.parse().map_err(|_| ResolveError::Contract)?,
        }),
    })
}

#[derive(Debug)]
struct Kernel {
    op: Op,
    choice: usize,
    scale: f64,
    rgba: TypeExpr,
}

impl PureKernel for Kernel {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        self.evaluate(inputs, outputs).map_err(|error| KernelFault {
            code: error.code().into(),
            message_key: error.message_key().into(),
            message: error.message(),
        })
    }
}

impl Kernel {
    fn evaluate(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelError> {
        use Op::*;
        if let (SplitRgba, [color], [r, g, b, a]) = (self.op, inputs, &mut *outputs) {
            let [red, green, blue, alpha] = self.channels(color)?.map(|n| Value::U32(u32::from(n)));
            (*r, *g, *b, *a) = (red, green, blue, alpha);
            return Ok(());
        }
        let [output] = outputs else {
            return Err(KernelError::InputContract);
        };
        let unit = || {
            [
                1_000_000,
                1_000_000_000,
                60_000_000_000,
                3_600_000_000_000,
                1,
            ][self.choice]
        };
        *output = match (self.op, inputs) {
            (Color, [color]) => {
                self.channels(color)?;
                color.clone()
            }
            (MakeRgba, [Value::U32(r), Value::U32(g), Value::U32(b), Value::U32(a)]) => {
                let [r, g, b, a] =
                    [r, g, b, a].map(|n| u8::try_from(*n).map_err(|_| KernelError::OutOfRange));
                self.color([r?, g?, b?, a?])
            }
            (ParseColor, [Value::Str(text)]) => self.color(parse_color(text)?),
            (FormatColor, [color]) => {
                let channels = self.channels(color)?;
                string(rgba_hex(channels))
            }
            (MakeDuration, [Value::I64(amount)]) => Value::Duration(
                amount
                    .checked_mul(unit())
                    .ok_or(KernelError::NumericOverflow)?,
            ),
            (DurationInUnits, [Value::Duration(duration)]) => Value::I64(duration / unit()),
            (ConvertTimeUnits, [Value::F64(amount)]) => {
                if !amount.is_finite() {
                    return Err(KernelError::Domain);
                }
                let converted = amount * self.scale;
                if !converted.is_finite() {
                    return Err(KernelError::NumericOverflow);
                }
                Value::F64(converted)
            }
            (AddDuration, [Value::Timestamp(at), Value::Duration(delta)]) => {
                Value::Timestamp(at.checked_add(*delta).ok_or(KernelError::NumericOverflow)?)
            }
            (SubtractDuration, [Value::Timestamp(at), Value::Duration(delta)]) => {
                Value::Timestamp(at.checked_sub(*delta).ok_or(KernelError::NumericOverflow)?)
            }
            (TimeBetween, [Value::Timestamp(start), Value::Timestamp(end)]) => Value::Duration(
                end.checked_sub(*start)
                    .ok_or(KernelError::NumericOverflow)?,
            ),
            (ParseTimestamp, [Value::Str(text)]) => {
                Value::Timestamp(parse_timestamp(text, self.choice)?)
            }
            (FormatTimestamp, [Value::Timestamp(at), Value::I64(offset)]) => {
                string(format_timestamp(*at, self.choice, *offset)?)
            }
            _ => return Err(KernelError::InputContract),
        };
        Ok(())
    }

    fn color(&self, color: [u8; 4]) -> Value {
        Value::Record(Arc::new(RecordRepr {
            ty: self.rgba.clone(),
            fields: Box::new(color.map(|n| Some(Value::U32(u32::from(n))))),
        }))
    }

    fn channels(&self, value: &Value) -> Result<[u8; 4], KernelError> {
        let Value::Record(record) = value else {
            return Err(KernelError::InputContract);
        };
        record.rgba_channels().ok_or(KernelError::OutOfRange)
    }
}

fn string(text: String) -> Value {
    Value::Str(Arc::new(text))
}

fn parse_color(text: &str) -> Result<[u8; 4], KernelError> {
    parse_rgba_hex(text).ok_or(KernelError::Domain)
}

fn parse_timestamp(text: &str, format: usize) -> Result<i64, KernelError> {
    if text.len() > 64 {
        return Err(KernelError::LimitExceeded);
    }
    let nanos = if format == 0 {
        text.parse::<jiff::Timestamp>()
            .map_err(|_| KernelError::Domain)?
            .as_nanosecond()
    } else {
        let decimals = STAMP_FRACTION_DIGITS[format];
        let negative = text.starts_with('-');
        let unsigned = text.strip_prefix('-').unwrap_or(text);
        let (whole, fraction) = unsigned
            .split_once('.')
            .map_or((unsigned, None), |(a, b)| (a, Some(b)));
        if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
            return Err(KernelError::Domain);
        }
        let whole: i128 = whole.parse().map_err(|_| KernelError::NumericOverflow)?;
        let scale = 10_i128.pow(decimals);
        let fraction = if let Some(fraction) = fraction {
            let len = u32::try_from(fraction.len()).map_err(|_| KernelError::Domain)?;
            if len == 0 || len > decimals || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                return Err(KernelError::Domain);
            }
            fraction.parse::<i128>().map_err(|_| KernelError::Domain)? * 10_i128.pow(decimals - len)
        } else {
            0
        };
        let magnitude = whole
            .checked_mul(scale)
            .and_then(|value| value.checked_add(fraction))
            .ok_or(KernelError::NumericOverflow)?;
        if negative { -magnitude } else { magnitude }
    };
    i64::try_from(nanos).map_err(|_| KernelError::NumericOverflow)
}

fn format_timestamp(nanos: i64, format: usize, offset_minutes: i64) -> Result<String, KernelError> {
    if format == 0 {
        let seconds = offset_minutes
            .checked_mul(60)
            .and_then(|n| i32::try_from(n).ok())
            .filter(|n| n.unsigned_abs() < 86_400)
            .ok_or(KernelError::OutOfRange)?;
        let offset =
            jiff::tz::Offset::from_seconds(seconds).map_err(|_| KernelError::OutOfRange)?;
        let at = jiff::Timestamp::from_nanosecond(i128::from(nanos))
            .map_err(|_| KernelError::OutOfRange)?;
        return Ok(at.display_with_offset(offset).to_string());
    }
    let digits = STAMP_FRACTION_DIGITS[format];
    let scale = 10_u64.pow(digits);
    let magnitude = nanos.unsigned_abs();
    let sign = if nanos < 0 { "-" } else { "" };
    let whole = magnitude / scale;
    let fraction = magnitude % scale;
    if fraction == 0 {
        return Ok(format!("{sign}{whole}"));
    }
    let decimal = format!("{fraction:0width$}", width = digits as usize);
    Ok(format!("{sign}{whole}.{}", decimal.trim_end_matches('0')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_unit_conversion_scales_and_rejects_nonfinite_values() {
        let kernel = Kernel {
            op: Op::ConvertTimeUnits,
            choice: 0,
            scale: 1000.0,
            rgba: RGBA_TYPE.parse().unwrap(),
        };
        let mut output = [Value::F64(0.0)];
        kernel.evaluate(&[Value::F64(0.125)], &mut output).unwrap();
        assert_eq!(output, [Value::F64(125.0)]);
        assert!(matches!(
            kernel.evaluate(&[Value::F64(f64::MAX)], &mut output),
            Err(KernelError::NumericOverflow)
        ));
        assert!(matches!(
            kernel.evaluate(&[Value::F64(f64::INFINITY)], &mut output),
            Err(KernelError::Domain)
        ));
    }

    #[test]
    fn timestamps_round_trip_before_epoch_and_at_signed_limits() {
        for format in 0..STAMP_FORMATS.len() {
            for time in [i64::MIN, -1_234_567_890, -1, 0, 1, i64::MAX] {
                let text = format_timestamp(time, format, -360).unwrap();
                assert_eq!(parse_timestamp(&text, format), Ok(time), "{text}");
            }
        }
        assert!(parse_timestamp("2026-02-30T12:00:00Z", 0).is_err());
        assert!(parse_timestamp("1.0000000001", 1).is_err());
    }

    #[test]
    fn hex_colors_preserve_alpha_and_support_opaque_rgb() {
        assert_eq!(parse_color("#0011ff"), Ok([0, 17, 255, 255]));
        assert_eq!(parse_color("#0011FF00"), Ok([0, 17, 255, 0]));
        assert!(parse_color("é12345").is_err());
    }
}
