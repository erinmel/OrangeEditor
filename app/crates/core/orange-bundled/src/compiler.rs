//! Trusted native adapter. The compiler itself has no bundled-provider dependency.
use crate::{
    BindingError, CollectionOperation, Comparison, FloatOperation, IntegerOperation, JsonOperation,
    MathPackage, NominalOperation, Operation, ResolvedCollection, ResolvedComparison,
    ResolvedConversion, ResolvedFloat, ResolvedInteger, ResolvedKernel, ResolvedText,
    TextOperation, resolve_core_conversion,
};
use orange_kernel::{
    BindingRequest, KernelBinding, KernelFault, PureKernel, PureKernelResolver, ResolveError,
};
use orange_model::Value;
use orange_model::catalog::{TypeArgs, intrinsic_id};
use orange_model::conversion::{ConverterConfig, ConverterProvider};
use orange_model::id::{PortKey, TypeExpr};
use std::sync::Arc;

fn ports(names: &[&str]) -> Result<Box<[PortKey]>, ResolveError> {
    names
        .iter()
        .map(|name| name.parse().map_err(|_| ResolveError::Contract))
        .collect()
}

/// The type the node's signature gives its port `name`.
fn port_type<'r>(
    request: &'r BindingRequest<'_>,
    name: &str,
) -> Result<&'r TypeExpr, ResolveError> {
    request
        .signature()
        .ports
        .iter()
        .find(|port| port.port_id.as_str() == name)
        .and_then(|port| port.ty.as_ref())
        .ok_or(ResolveError::Contract)
}

/// A kernel that cannot be bound: its type is not one it handles, or it is missing.
fn unbound(error: BindingError) -> ResolveError {
    match error {
        BindingError::UnsupportedType => ResolveError::UnsupportedType,
        _ => ResolveError::Unavailable,
    }
}

impl PureKernelResolver for MathPackage {
    fn resolve_binding(&self, request: &BindingRequest<'_>) -> Result<KernelBinding, ResolveError> {
        let definition = request.definition();
        if definition.id.as_str() == intrinsic_id::CONVERT
            && definition.version == 1
            && request.config_version() == 1
            && request.provider().is_none()
        {
            let config = ConverterConfig::from_config(request.config())
                .map_err(|_| ResolveError::Contract)?;
            let conversion = resolve_core_conversion(
                request.snapshot().clone(),
                &config,
                &ConverterProvider::Core,
            )
            .map_err(|_| ResolveError::Unavailable)?;
            return Ok(KernelBinding {
                inputs: ports(&["value"])?,
                outputs: ports(if conversion.lossy() {
                    &["result", "loss"]
                } else {
                    &["result"]
                })?,
                kernel: Arc::new(Native::Conversion(conversion)),
            });
        }
        if let Some(binding) = crate::presentation_values::bind(self, request) {
            return binding;
        }
        let provider = request.provider().ok_or(ResolveError::Unavailable)?;
        // A shared operation (Negate, Absolute, Minimum, Maximum, Clamp) binds decimals
        // to its float kernel below and whole numbers here.
        let decimal = |port: &str| {
            port_type(request, port)
                .is_ok_and(|ty| matches!(ty.as_str(), "core/f32@1" | "core/f64@1"))
        };
        if let Some(operation) = IntegerOperation::from_definition_id(definition.id.as_str())
            && !(operation.shared() && decimal(operation.input_ports()[0]))
        {
            let ty = port_type(request, operation.input_ports()[0])?;
            let kernel = self
                .resolve_integer(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    ty,
                    request.config(),
                )
                .map_err(unbound)?;
            return Ok(KernelBinding {
                inputs: ports(operation.input_ports())?,
                outputs: ports(&[operation.output_port()])?,
                kernel: Arc::new(Native::Integer(kernel)),
            });
        }
        if let Some(operation) = JsonOperation::from_definition_id(definition.id.as_str()) {
            let kernel = self
                .resolve_json(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    request.config(),
                )
                .map_err(|_| ResolveError::Unavailable)?;
            let inputs: Vec<&str> = operation.inputs().iter().map(|(name, _)| *name).collect();
            return Ok(KernelBinding {
                inputs: ports(&inputs)?,
                outputs: ports(&["result"])?,
                kernel: Arc::new(Native::Json(kernel)),
            });
        }
        if NominalOperation::from_definition_id(definition.id.as_str()).is_some() {
            let kernel = self
                .resolve_nominal(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    request.config(),
                )
                .map_err(unbound)?;
            let inputs: Vec<&str> = kernel.input_ports().iter().map(String::as_str).collect();
            return Ok(KernelBinding {
                inputs: ports(&inputs)?,
                outputs: ports(&["result"])?,
                kernel: Arc::new(Native::Nominal(kernel)),
            });
        }
        if let Some(operation) = CollectionOperation::from_definition_id(definition.id.as_str()) {
            let container_port = match operation {
                CollectionOperation::BuildList
                | CollectionOperation::BuildMap
                | CollectionOperation::OptionSome
                | CollectionOperation::OptionNone => operation.output_port(),
                _ => operation.inputs()[0].0,
            };
            let container = port_type(request, container_port)?;
            let kernel = self
                .resolve_collection(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    container,
                    request.config(),
                )
                .map_err(unbound)?;
            let inputs = request
                .signature()
                .ports
                .iter()
                .filter(|port| port.direction == orange_model::document::PortDirection::Input)
                .flat_map(|port| request.port_keys(port))
                .collect();
            return Ok(KernelBinding {
                inputs,
                outputs: ports(&[operation.output_port()])?,
                kernel: Arc::new(Native::Collection(kernel)),
            });
        }
        if let Some(operation) = TextOperation::from_definition_id(definition.id.as_str()) {
            let kernel = self
                .resolve_text(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    request.config(),
                )
                .map_err(|_| ResolveError::Unavailable)?;
            let inputs: Vec<&str> = operation.inputs().iter().map(|(name, _)| *name).collect();
            return Ok(KernelBinding {
                inputs: ports(&inputs)?,
                outputs: ports(&[operation.output_port()])?,
                kernel: Arc::new(Native::Text(kernel)),
            });
        }
        if let Some(operation) = FloatOperation::from_definition_id(definition.id.as_str()) {
            let ty = port_type(request, operation.input_ports()[0])?;
            let kernel = self
                .resolve_float(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    ty,
                    request.config(),
                )
                .map_err(unbound)?;
            return Ok(KernelBinding {
                inputs: ports(operation.input_ports())?,
                outputs: ports(&[operation.output_port()])?,
                kernel: Arc::new(Native::Float(kernel)),
            });
        }
        if let Some(comparison) = Comparison::from_definition_id(definition.id.as_str()) {
            let [a, b] = comparison.input_ports() else {
                return Err(ResolveError::Contract);
            };
            let ty = port_type(request, a)?;
            if port_type(request, b)? != ty {
                return Err(ResolveError::Contract);
            }
            let resolved = self
                .resolve_comparison(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    ty,
                    request.config(),
                )
                .map_err(unbound)?;
            return Ok(KernelBinding {
                inputs: ports(comparison.input_ports())?,
                outputs: ports(&[comparison.output_port()])?,
                kernel: Arc::new(Native::Comparison(resolved)),
            });
        }
        let operation = Operation::from_definition_id(definition.id.as_str())
            .ok_or(ResolveError::Unavailable)?;
        let inputs = operation.input_ports();
        let args: TypeArgs = if !operation.is_numeric() {
            Default::default()
        } else {
            [("T".into(), port_type(request, inputs[0])?.clone())].into()
        };
        let kernel = MathPackage::resolve(
            self,
            request.snapshot().clone(),
            provider,
            definition,
            request.config_version(),
            &args,
            request.config(),
        )
        .map_err(|_| ResolveError::Unavailable)?;
        Ok(KernelBinding {
            inputs: ports(inputs)?,
            outputs: ports(&[operation.output_port()])?,
            kernel: Arc::new(Native::Math(kernel)),
        })
    }
}

#[derive(Debug)]
enum Native {
    Math(ResolvedKernel),
    Integer(ResolvedInteger),
    Float(ResolvedFloat),
    Text(ResolvedText),
    Collection(ResolvedCollection),
    Nominal(crate::ResolvedNominal),
    Json(crate::ResolvedJson),
    Comparison(ResolvedComparison),
    Conversion(ResolvedConversion),
}
const CONTRACT: KernelFault = KernelFault {
    code: orange_kernel::FaultText::Static("core/InternalFault"),
    message_key: orange_kernel::FaultText::Static("core/kernel-input-contract"),
    message: "Kernel inputs violate the resolved contract.",
};
impl PureKernel for Native {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        let evaluated = match self {
            Self::Integer(kernel) => kernel.evaluate(inputs),
            Self::Json(kernel) => kernel.evaluate(inputs),
            Self::Nominal(kernel) => kernel.evaluate(inputs),
            Self::Collection(kernel) => kernel.evaluate(inputs),
            Self::Text(kernel) => kernel.evaluate(inputs),
            Self::Float(kernel) => kernel.evaluate(inputs),
            Self::Math(kernel) => kernel.evaluate(inputs),
            Self::Comparison(comparison) => {
                let ([a, b], [output]) = (inputs, outputs) else {
                    return Err(CONTRACT);
                };
                *output = Value::Bool(comparison.evaluate(a, b).map_err(|_| CONTRACT)?);
                return Ok(());
            }
            Self::Conversion(conversion) => {
                let [input] = inputs else {
                    return Err(CONTRACT);
                };
                let value = conversion.evaluate(input).map_err(|error| KernelFault {
                    code: error.code().into(),
                    message_key: error.message_key().into(),
                    message: error.message(),
                })?;
                match (outputs, value.loss) {
                    ([result], None) => *result = value.result,
                    ([result, loss], Some(metadata)) => {
                        *result = value.result;
                        *loss = metadata;
                    }
                    _ => return Err(CONTRACT),
                }
                return Ok(());
            }
        };
        let [output] = outputs else {
            return Err(CONTRACT);
        };
        *output = evaluated.map_err(|error| {
            if matches!(self, Self::Text(kernel) if kernel.operation() == TextOperation::Format)
                && error == crate::KernelError::OutOfRange
            {
                return KernelFault {
                    code: "org.orange.text/MissingFormatArgument".into(),
                    message_key: "org.orange.text/missing-format-argument".into(),
                    message: "The template refers to an argument that is missing.",
                };
            }
            KernelFault {
                code: error.code().into(),
                message_key: error.message_key().into(),
                message: match (self, error) {
                    (Self::Math(_), crate::KernelError::InputContract) => CONTRACT.message,
                    (_, other) => other.message(),
                },
            }
        })?;
        Ok(())
    }
}
