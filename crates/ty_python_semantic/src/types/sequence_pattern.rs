use ruff_python_ast::name::Name;

use crate::Db;
use crate::types::callable::{CallableFunctionProvenance, CallableTypeKind};
use crate::types::signatures::CallableSignature;
use crate::types::tuple::{TupleSpec, TupleSpecBuilder, TupleType};
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
/// and element types. The protocol also carries the equivalent tuple shape so
/// tuple intersections and negative branches can stay precise.
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

    let Some(tuple) = TupleType::heterogeneous(db, element_types.iter().copied()) else {
        return Type::Never;
    };
    let protocol = Type::exact_sequence_protocol_with_methods(db, methods, tuple);

    IntersectionBuilder::new(db)
        .add_positive(sequence_pattern_type(db))
        .add_positive(protocol)
        .build()
}

/// Intersect a concrete tuple instance with an exact-sequence protocol.
///
/// Returns `None` when `ty` is not a tuple-shaped nominal instance,
/// `Some(Never)` when the tuple shape is disjoint from the protocol, and the
/// refined tuple type otherwise.
pub(crate) fn refine_tuple_with_exact_sequence_protocol<'db>(
    db: &'db dyn Db,
    ty: Type<'db>,
    protocol_tuple: TupleType<'db>,
) -> Option<Type<'db>> {
    let Type::NominalInstance(instance) = ty else {
        return None;
    };

    let tuple = instance.own_tuple_spec(db)?;
    let Some(refined) =
        TupleSpecBuilder::from(tuple.as_ref()).intersect(db, protocol_tuple.tuple(db))
    else {
        return Some(Type::Never);
    };

    Some(Type::tuple(TupleType::new(db, &refined.build())))
}

/// Subtract an exact-sequence protocol from a fixed-length tuple type.
///
/// The returned alternatives are the remaining tuple shapes. An empty vector
/// means the tuple was fully covered by the protocol; `None` means `ty` is not
/// a fixed tuple and the caller should use a less precise fallback.
pub(crate) fn subtract_exact_sequence_protocol_from_tuple<'db>(
    db: &'db dyn Db,
    ty: Type<'db>,
    protocol_tuple: TupleType<'db>,
) -> Option<Vec<Type<'db>>> {
    let TupleSpec::Fixed(protocol_tuple) = protocol_tuple.tuple(db) else {
        return None;
    };

    let Type::NominalInstance(instance) = ty else {
        return None;
    };
    let tuple = instance.own_tuple_spec(db)?;

    let TupleSpec::Fixed(tuple) = tuple.as_ref() else {
        return None;
    };

    if tuple.len() != protocol_tuple.len() {
        return Some(vec![ty]);
    }

    let mut alternatives = Vec::new();
    let mut is_subtype = true;

    for (index, (element, protocol_element)) in tuple
        .all_elements()
        .iter()
        .zip(protocol_tuple.all_elements())
        .enumerate()
    {
        if element.is_disjoint_from(db, *protocol_element) {
            return Some(vec![ty]);
        }

        if element.is_subtype_of(db, *protocol_element) {
            continue;
        }

        is_subtype = false;

        let remaining_element = IntersectionBuilder::new(db)
            .add_positive(*element)
            .add_negative(*protocol_element)
            .build();

        if remaining_element.is_never() {
            continue;
        }

        let mut elements = tuple.all_elements().to_vec();
        elements[index] = remaining_element;
        alternatives.push(Type::tuple(TupleType::heterogeneous(db, elements)));
    }

    if is_subtype {
        Some(vec![])
    } else {
        Some(alternatives)
    }
}
