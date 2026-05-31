use ruff_python_ast::name::Name;

use crate::Db;
use crate::types::callable::{CallableFunctionProvenance, CallableTypeKind};
use crate::types::signatures::CallableSignature;
use crate::types::{
    CallableType, IntersectionBuilder, KnownClass, Parameter, Parameters, Signature, Type,
};

pub(crate) fn sequence_pattern_type(db: &dyn Db) -> Type<'_> {
    IntersectionBuilder::new(db)
        .add_positive(KnownClass::Sequence.to_instance(db).top_materialization(db))
        // `str`, `bytes`, and `bytearray` are sequences, but Python sequence
        // patterns explicitly do not match them or their subclasses.
        .add_negative(KnownClass::Str.to_instance(db))
        .add_negative(KnownClass::Bytes.to_instance(db))
        .add_negative(KnownClass::Bytearray.to_instance(db))
        .build()
}

/// Build the structural type used for a fixed-length sequence pattern.
///
/// For a pattern like:
///
/// ```python
/// match value:
///     case [int(), str()]:
///         ...
/// ```
///
/// this returns the sequence-pattern runtime type plus a synthesized protocol
/// whose `__len__` and indexed `__getitem__` methods encode the fixed length
/// and element types.
pub(crate) fn exact_sequence_pattern_type<'db>(
    db: &'db dyn Db,
    element_types: &[Type<'db>],
) -> Type<'db> {
    let Ok(length) = i64::try_from(element_types.len()) else {
        return sequence_pattern_type(db);
    };

    let length_type = Type::int_like_literal(db, length);

    let self_parameter = || Parameter::positional_only(Some(Name::new_static("self")));

    let len_signature = Signature::new(Parameters::new(db, [self_parameter()]), length_type);
    let len_method = CallableType::function_like(db, len_signature);

    let mut methods = vec![("__len__", len_method)];

    if !element_types.is_empty() {
        let getitem_overloads = (0..length).zip(element_types).map(|(index, element_type)| {
            Signature::new(
                Parameters::new(
                    db,
                    [
                        self_parameter(),
                        Parameter::positional_only(Some(Name::new_static("index")))
                            .with_annotated_type(Type::int_literal(index)),
                    ],
                ),
                *element_type,
            )
        });

        methods.push((
            "__getitem__",
            CallableType::new(
                db,
                CallableSignature::from_overloads(getitem_overloads),
                CallableTypeKind::FunctionLike,
                CallableFunctionProvenance::None,
            ),
        ));
    }

    let protocol = Type::protocol_with_methods(db, methods);

    IntersectionBuilder::new(db)
        .add_positive(sequence_pattern_type(db))
        .add_positive(protocol)
        .build()
}
