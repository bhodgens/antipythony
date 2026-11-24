"""pydantic_core.core_schema namespace shim (F-STRUCT-1 stage).

The probe extension registers core_schema as a string marker; pydantic
2.14.1 reads ~100 schema-constructor/TYPE names off it via
`core_schema.X` attribute access. CPython's real pydantic_core serves the
full namespace. This shim (imported by the probe driver before pydantic)
REPLACES the marker with a module-type object answering every name pydantic
touches — schema fns as inert markers (the BaseModel dataclass chain only
needs the NAMES to resolve), TYPE names as themselves.

The ce shim's str marker stays (probe gate byte-stability); this module
upgrades it in place.
"""
import sys
import types

pc = sys.modules.get('pydantic_core')
cs = getattr(pc, 'core_schema', None) if pc else None

SCHEMA_FNS = [
    'any_schema', 'arguments_parameter', 'arguments_schema',
    'bool_schema', 'bytes_schema', 'call_schema', 'callable_schema',
    'chain_schema', 'complex_schema', 'computed_field', 'counter_schema',
    'custom_error_schema', 'dataclass_args_schema', 'dataclass_field',
    'dataclass_schema', 'date_schema', 'datetime_schema', 'decimal_schema',
    'definition_reference_schema', 'definitions_schema', 'deque_schema',
    'dict_schema', 'ellipsis_schema', 'enum_schema', 'float_schema',
    'frozendict_schema', 'frozenset_schema', 'generator_schema',
    'int_schema', 'is_instance_schema', 'is_subclass_schema',
    'json_or_python_schema', 'lax_or_strict_schema', 'list_schema',
    'literal_schema', 'missing_sentinel_schema', 'model_field',
    'model_fields_schema', 'model_schema', 'multi_host_url_schema',
    'named_tuple_field', 'named_tuple_schema', 'no_info_after_validator_function',
    'no_info_before_validator_function', 'no_info_plain_validator_function',
    'no_info_wrap_validator_function', 'none_schema', 'nullable_schema',
    'ordered_dict_schema', 'plain_serializer_function_ser_schema',
    'set_schema', 'simple_ser_schema', 'str_schema', 'time_schema',
    'timeedelta_schema', 'to_string_ser_schema', 'tuple_schema',
    'typed_dict_field', 'typed_dict_schema', 'union_schema', 'url_schema',
    'uuid_schema', 'with_default_schema', 'with_info_after_validator_function',
    'with_info_before_validator_function', 'with_info_plain_validator_function',
    'with_info_wrap_validator_function', 'wrap_serializer_function_ser_schema',
]

TYPE_NAMES = [
    'ArgumentsParameter', 'ArgumentsSchema', 'ArgumentsV', 'CallSchema',
    'ComputedField', 'CoreSchema', 'DataclassField', 'DefinitionsSchema',
    'ExtraBehavior', 'GeneratorSchema', 'ModelField', 'NamedTupleField',
    'SerializationInfo', 'SerializerFunctionWrapHandler', 'SerSchema',
    'TypedDictField', 'VarKwargsMode', 'WhenUsed',
    'WithInfoValidatorFunction', 'DefinitionReferenceSchema',
]


def _protocol_type(name):
    # pydantic subclasses these as Protocol bases
    # ("Protocols can only inherit from other protocols" otherwise) — each
    # shim TYPE name is itself a typing_extensions.Protocol subclass.
    import typing
    import typing_extensions as te

    base = getattr(te, 'Protocol', typing.Protocol)
    ns = {'__call__': lambda self, *a, **k: None, '__slots__': ()}
    return type(name, (base,), ns)

OTHER = ['CoreConfig', 'MISSING', 'PydanticUndefined', 'Some',
         'PydanticSerializationUnexpectedValue']


def _marker(name):
    def fn(*args, **kwargs):
        return None
    fn.__name__ = name
    return fn


def upgrade():
    ns = types.ModuleType('pydantic_core.core_schema')
    for n in SCHEMA_FNS:
        setattr(ns, n, _marker(n))
    for n in TYPE_NAMES:
        setattr(ns, n, _protocol_type(n))
    # Handler/info types pydantic Protocol-subclasses by NAME access:
    for n in ['ValidatorFunctionWrapHandler', 'ValidationInfo']:
        setattr(ns, n, _protocol_type(n))
    for n in OTHER:
        setattr(ns, n, n)  # inert string marker
    # pydantic unions these NAMES in TypeVar bounds / annotations
    # (`SchemaValidator | SchemaSerializer | PluggableSchemaValidator`):
    # the ce shim's STR markers can't take `|`. Upgrade pydantic_core's
    # class-shaped names to real (opaque) classes. ArgsKwargs too
    # (_dataclasses imports it from pydantic_core).
    if pc is not None:
        for n in ['SchemaValidator', 'SchemaSerializer', 'CoreConfig',
                  'CoreSchema', 'PydanticCustomError', 'PydanticKnownError',
                  'PydanticOmit', 'PydanticUndefinedType', 'Some', 'Url',
                  'MultiHostUrl', 'PluggableSchemaValidator', 'ArgsKwargs',
                  'ValidationError']:
            if isinstance(getattr(pc, n, None), str):
                setattr(pc, n, type(n, (), {'__slots__': ()}))
        setattr(pc, 'core_schema', ns)
    sys.modules['pydantic_core.core_schema'] = ns
    return ns


upgrade()
