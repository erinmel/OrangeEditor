//! Trusted native adapter. The compiler itself has no bundled-provider dependency.
use crate::{
    BindingError, CollectionOperation, Comparison, FloatOperation, IntegerOperation, MathPackage,
    Operation, ResolvedCollection, ResolvedComparison, ResolvedConversion, ResolvedFloat,
    ResolvedInteger, ResolvedKernel, ResolvedText, TextOperation, resolve_core_conversion,
};
use orange_compiler::{
    BindingRequest, KernelBinding, KernelFault, PureKernel, PureKernelResolver, ResolveError,
};
use orange_model::Value;
use orange_model::catalog::{TypeArgs, intrinsic_id};
use orange_model::conversion::{ConverterConfig, ConverterProvider};
use orange_model::id::PortKey;
use std::sync::Arc;

fn ports(names: &[&str]) -> Result<Box<[PortKey]>, ResolveError> {
    names
        .iter()
        .map(|name| name.parse().map_err(|_| ResolveError::Contract))
        .collect()
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
        let provider = request.provider().ok_or(ResolveError::Unavailable)?;
        if let Some(operation) = IntegerOperation::from_definition_id(definition.id.as_str()) {
            let ty = request
                .signature()
                .ports
                .iter()
                .find(|port| port.port_id.as_str() == operation.input_ports()[0])
                .and_then(|port| port.ty.as_ref())
                .ok_or(ResolveError::Contract)?;
            let kernel = self
                .resolve_integer(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    ty,
                    request.config(),
                )
                .map_err(|error| match error {
                    BindingError::UnsupportedType => ResolveError::UnsupportedType,
                    _ => ResolveError::Unavailable,
                })?;
            return Ok(KernelBinding {
                inputs: ports(operation.input_ports())?,
                outputs: ports(&[operation.output_port()])?,
                kernel: Arc::new(Native::Integer(kernel)),
            });
        }
        if let Some(operation) = CollectionOperation::from_definition_id(definition.id.as_str()) {
            let first = operation.inputs()[0].0;
            let container = request
                .signature()
                .ports
                .iter()
                .find(|port| port.port_id.as_str() == first)
                .and_then(|port| port.ty.as_ref())
                .ok_or(ResolveError::Contract)?;
            let kernel = self
                .resolve_collection(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    container,
                    request.config(),
                )
                .map_err(|error| match error {
                    BindingError::UnsupportedType => ResolveError::UnsupportedType,
                    _ => ResolveError::Unavailable,
                })?;
            let inputs: Vec<&str> = operation.inputs().iter().map(|(name, _)| *name).collect();
            return Ok(KernelBinding {
                inputs: ports(&inputs)?,
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
            let ty = request
                .signature()
                .ports
                .iter()
                .find(|port| port.port_id.as_str() == operation.input_ports()[0])
                .and_then(|port| port.ty.as_ref())
                .ok_or(ResolveError::Contract)?;
            let kernel = self
                .resolve_float(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    ty,
                    request.config(),
                )
                .map_err(|error| match error {
                    BindingError::UnsupportedType => ResolveError::UnsupportedType,
                    _ => ResolveError::Unavailable,
                })?;
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
            let ty = |name: &str| {
                request
                    .signature()
                    .ports
                    .iter()
                    .find(|port| port.port_id.as_str() == name)
                    .and_then(|port| port.ty.clone())
            };
            let ty = ty(a)
                .filter(|ty_a| ty(b).as_ref() == Some(ty_a))
                .ok_or(ResolveError::Contract)?;
            let resolved = self
                .resolve_comparison(
                    request.snapshot().clone(),
                    provider,
                    definition,
                    request.config_version(),
                    &ty,
                    request.config(),
                )
                .map_err(|error| match error {
                    BindingError::UnsupportedType => ResolveError::UnsupportedType,
                    _ => ResolveError::Unavailable,
                })?;
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
            let ty = request
                .signature()
                .ports
                .iter()
                .find(|port| port.port_id.as_str() == inputs[0])
                .and_then(|port| port.ty.clone())
                .ok_or(ResolveError::Contract)?;
            [("T".into(), ty)].into()
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
    Comparison(ResolvedComparison),
    Conversion(ResolvedConversion),
}
const CONTRACT: KernelFault = KernelFault {
    code: "core/InternalFault",
    message_key: "core/kernel-input-contract",
    message: "Kernel inputs violate the resolved contract.",
};
impl PureKernel for Native {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        match self {
            Self::Integer(kernel) => {
                let [output] = outputs else {
                    return Err(CONTRACT);
                };
                *output = kernel.evaluate(inputs).map_err(|error| KernelFault {
                    code: error.code(),
                    message_key: error.message_key(),
                    message: error.message(),
                })?;
            }
            Self::Collection(kernel) => {
                let [output] = outputs else {
                    return Err(CONTRACT);
                };
                *output = kernel.evaluate(inputs).map_err(|error| KernelFault {
                    code: error.code(),
                    message_key: error.message_key(),
                    message: error.message(),
                })?;
            }
            Self::Text(kernel) => {
                let [output] = outputs else {
                    return Err(CONTRACT);
                };
                *output = kernel.evaluate(inputs).map_err(|error| KernelFault {
                    code: error.code(),
                    message_key: error.message_key(),
                    message: error.message(),
                })?;
            }
            Self::Float(kernel) => {
                let [output] = outputs else {
                    return Err(CONTRACT);
                };
                *output = kernel.evaluate(inputs).map_err(|error| KernelFault {
                    code: error.code(),
                    message_key: error.message_key(),
                    message: error.message(),
                })?;
            }
            Self::Math(kernel) => {
                let [output] = outputs else {
                    return Err(CONTRACT);
                };
                *output = kernel.evaluate(inputs).map_err(|error| KernelFault {
                    code: error.code(),
                    message_key: error.message_key(),
                    message: match error {
                        crate::KernelError::InputContract => CONTRACT.message,
                        other => other.message(),
                    },
                })?;
            }
            Self::Comparison(comparison) => {
                let ([a, b], [output]) = (inputs, outputs) else {
                    return Err(CONTRACT);
                };
                *output = Value::Bool(comparison.evaluate(a, b).map_err(|_| CONTRACT)?);
            }
            Self::Conversion(conversion) => {
                let [input] = inputs else {
                    return Err(CONTRACT);
                };
                let value = conversion.evaluate(input).map_err(|error| KernelFault {
                    code: error.code(),
                    message_key: error.message_key(),
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
            }
        }
        Ok(())
    }
}
