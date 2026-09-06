fn parse_create_json<'de, T>(json: &'de str) -> Result<T, SessionError>
where
    T: serde::Deserialize<'de>,
{
    serde_json::from_str(json)
        .map_err(|error| SessionError::from(BoundaryError::parse("CONFIG_INVALID", error)))
}

fn admit_create_wire_bytes(actual: usize) -> Result<(), SessionError> {
    if actual > CREATE_WIRE_MAX_BYTES {
        return Err(
            BoundaryError::limit("INPUT_LIMIT_EXCEEDED", CREATE_WIRE_MAX_BYTES, actual).into(),
        );
    }
    Ok(())
}

fn admit_create_envelope_bytes(actual: usize) -> Result<(), SessionError> {
    if actual > CREATE_ENVELOPE_MAX_BYTES {
        return Err(BoundaryError::limit(
            "INPUT_LIMIT_EXCEEDED",
            CREATE_ENVELOPE_MAX_BYTES,
            actual,
        )
        .into());
    }
    Ok(())
}

fn resolve_configured_create_schema(
    config: &EditorSessionConfig,
) -> Result<crate::schema::Schema, SessionError> {
    let Some(schema_json) = config.schema_json.as_deref() else {
        return super::render::resolve_create_schema(&None);
    };
    let input = BoundedInput::new(schema_json, InputKind::Config, &config.resource_limits)?;
    let container_limit = crate::schema::MAX_SCHEMA_METADATA_DEPTH
        .checked_add(16)
        .ok_or_else(|| BoundaryError::new("SCHEMA_INVALID", "schema depth limit overflow"))?;
    let schema = parse_json_value_stack_safe(
        input.as_str(),
        container_limit,
        crate::schema::MAX_SCHEMA_METADATA_DEPTH,
        "SCHEMA_INVALID",
        "SCHEMA_INVALID",
    )?;
    crate::schema::Schema::from_json_with_limits(schema.as_value(), &config.resource_limits)
        .map_err(SessionError::from)
}

/// A fully bounded, local-only document import shared by editor admission and
/// immutable viewer compilation. It intentionally stops before any registry,
/// Yjs, collaboration, or editor-handle allocation.
pub(crate) struct ResolvedLocalDocument {
    pub document: crate::model::Document,
    pub schema: crate::schema::Schema,
}

pub(crate) fn resolve_local_document(
    config_json: &str,
    source_kind: FfiViewerSourceKind,
    source: &str,
) -> Result<ResolvedLocalDocument, SessionError> {
    let (config, schema) = resolve_local_config(config_json)?;
    let input_kind = match source_kind {
        FfiViewerSourceKind::Json => InputKind::DocumentJson,
        FfiViewerSourceKind::Html => InputKind::Html,
    };
    let input = BoundedInput::new(source, input_kind, &config.resource_limits)?;
    let document = match source_kind {
        FfiViewerSourceKind::Json => {
            let depth_limit = crate::boundary::document_json_container_depth_limit(
                config.resource_limits.max_document_depth,
            )?;
            let value = parse_json_value_stack_safe(
                input.as_str(),
                depth_limit,
                config.resource_limits.max_document_depth,
                "DOCUMENT_LIMIT_EXCEEDED",
                "DOCUMENT_INVALID",
            )?;
            let document = crate::serialize::from_prosemirror_json_with_limits(
                value.as_value(),
                &schema,
                crate::serialize::UnknownTypeMode::Preserve,
                &config.resource_limits,
            )
            .map_err(viewer_json_parse_error)?;
            crate::yrs_engine::admit_local_import_document(
                document,
                &schema,
                &config.resource_limits,
                &config.editing_limits,
                Some(input.as_str().len()),
            )
            .map_err(SessionError::from)?
        }
        FfiViewerSourceKind::Html => {
            let document = crate::serialize::from_html_with_limits(
                input.as_str(),
                &schema,
                &crate::serialize::FromHtmlOptions {
                    strict: false,
                    allow_base64_images: config.allow_base64_images,
                },
                &config.resource_limits,
            )
            .map_err(viewer_html_parse_error)?;
            crate::yrs_engine::admit_local_import_document(
                document,
                &schema,
                &config.resource_limits,
                &config.editing_limits,
                None,
            )
            .map_err(SessionError::from)?
        }
    };

    Ok(ResolvedLocalDocument { document, schema })
}

fn resolve_local_empty_document(config_json: &str) -> Result<ResolvedLocalDocument, SessionError> {
    let (config, schema) = resolve_local_config(config_json)?;
    let document = schema
        .default_document()
        .map_err(|error| SessionError::new(ErrorDomain::Document, "DOCUMENT_INVALID", error))?;
    let document = crate::yrs_engine::admit_local_import_document(
        document,
        &schema,
        &config.resource_limits,
        &config.editing_limits,
        None,
    )
    .map_err(SessionError::from)?;

    Ok(ResolvedLocalDocument { document, schema })
}

fn resolve_local_config(
    config_json: &str,
) -> Result<(EditorSessionConfig, crate::schema::Schema), SessionError> {
    admit_create_wire_bytes(config_json.len())?;
    admit_create_retained_envelope(config_json)?;
    let envelope: CreateEnvelope<'_> = parse_create_json(config_json)?;
    let initialization_probe: InitializationProbe =
        parse_create_json(envelope.initialization.get())?;
    let (config, room_bound) = build_config(envelope, initialization_probe, None)?;
    if room_bound {
        return Err(config_invalid(
            None,
            "viewer compilation requires a local initialization configuration",
        ));
    }
    let schema = resolve_configured_create_schema(&config)?;
    Ok((config, schema))
}

fn viewer_json_parse_error(error: crate::serialize::JsonParseError) -> SessionError {
    match error {
        crate::serialize::JsonParseError::ResourceLimit { limit, actual } => {
            BoundaryError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, actual).into()
        }
        error => SessionError::new(ErrorDomain::Document, "DOCUMENT_INVALID", error.to_string()),
    }
}

fn viewer_html_parse_error(error: crate::serialize::ParseError) -> SessionError {
    match error {
        crate::serialize::ParseError::ResourceLimit { limit, actual } => {
            BoundaryError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, actual).into()
        }
        error => SessionError::new(ErrorDomain::Document, "DOCUMENT_INVALID", error.to_string()),
    }
}

fn build_config(
    envelope: CreateEnvelope<'_>,
    initialization_probe: InitializationProbe,
    snapshot_state: Option<Vec<u8>>,
) -> Result<(EditorSessionConfig, bool), SessionError> {
    let CreateEnvelope {
        schema,
        fragment_name,
        initialization: initialization_json,
        policy,
        limits,
    } = envelope;
    let LimitsEnvelope {
        resource,
        editing,
        collaboration,
    } = limits
        .map(|limits| parse_create_json(limits.get()))
        .transpose()?
        .unwrap_or_default();
    let resource_limits = ResourceLimits::resolve(resource.unwrap_or_default())?;
    let editing_limits = EditingLimits::resolve(editing.unwrap_or_default())?;
    let collaboration_limits = CollaborationLimits::resolve(collaboration.unwrap_or_default())?;
    let PolicyEnvelope {
        max_length,
        read_only,
        input_filter,
        allow_base64_images,
    } = policy
        .map(|policy| parse_create_json(policy.get()))
        .transpose()?
        .unwrap_or_default();
    let fragment_name = fragment_name
        .map(|fragment_name| parse_create_json(fragment_name.get()))
        .transpose()?;
    if !matches!(initialization_probe.kind, InitializationKind::Room) && snapshot_state.is_some() {
        return Err(config_invalid(
            None,
            "snapshot state bytes require a room initialization with snapshot metadata",
        ));
    }
    let schema_json = schema
        .map(|schema| materialize_raw_payload(schema, InputKind::Config, &resource_limits))
        .transpose()?;
    let (initialization, room_bound) = match initialization_probe.kind {
        InitializationKind::LocalEmpty => {
            let _: LocalEmptyInitialization = parse_create_json(initialization_json.get())?;
            (
                EditorInitialization::Local {
                    initial_content: InitialContent::Empty,
                },
                false,
            )
        }
        InitializationKind::LocalJson => {
            let initialization: LocalJsonInitialization<'_> =
                parse_create_json(initialization_json.get())?;
            let json = materialize_raw_payload(
                initialization.json,
                InputKind::DocumentJson,
                &resource_limits,
            )?;
            (
                EditorInitialization::Local {
                    initial_content: InitialContent::Json(json),
                },
                false,
            )
        }
        InitializationKind::LocalHtml => {
            let initialization: LocalHtmlInitialization<'_> =
                parse_create_json(initialization_json.get())?;
            let html = materialize_html(initialization.html, &resource_limits)?;
            (
                EditorInitialization::Local {
                    initial_content: InitialContent::Html(html),
                },
                false,
            )
        }
        InitializationKind::Room => {
            let initialization: RoomInitialization = parse_create_json(initialization_json.get())?;
            let snapshot = match (initialization.snapshot, snapshot_state) {
                (Some(metadata), Some(encoded_state)) => {
                    admit_snapshot_state(&encoded_state, &resource_limits)?;
                    Some(metadata.into_snapshot(encoded_state))
                }
                (None, None) => None,
                _ => {
                    return Err(config_invalid(
                        None,
                        "room snapshot metadata and snapshot state bytes must arrive together",
                    ));
                }
            };
            (
                EditorInitialization::Room {
                    scope: DocumentScope {
                        document_id: initialization.document_id,
                        lineage_id: initialization.lineage_id,
                    },
                    snapshot,
                },
                true,
            )
        }
    };
    Ok((
        EditorSessionConfig {
            schema_json,
            fragment_name: fragment_name.unwrap_or_else(|| "prosemirror".into()),
            initialization,
            resource_limits,
            editing_limits,
            collaboration_limits,
            max_length,
            read_only: read_only.unwrap_or(false),
            input_filter,
            allow_base64_images: allow_base64_images.unwrap_or(false),
        },
        room_bound,
    ))
}

fn materialize_raw_payload(
    raw: &RawValue,
    kind: InputKind,
    limits: &ResourceLimits,
) -> Result<String, SessionError> {
    let input = BoundedInput::new(raw.get(), kind, limits)?;
    Ok(input.as_str().to_owned())
}

fn materialize_html(raw: &RawValue, limits: &ResourceLimits) -> Result<String, SessionError> {
    let json = raw.get();
    if let Some(unescaped) = json
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .filter(|value| !value.as_bytes().contains(&b'\\'))
    {
        let input = BoundedInput::new(unescaped, InputKind::Html, limits)?;
        return Ok(input.as_str().to_owned());
    }
    let html: String = parse_create_json(json)?;
    BoundedInput::new(&html, InputKind::Html, limits)?;
    Ok(html)
}

fn admit_snapshot_state(encoded_state: &[u8], limits: &ResourceLimits) -> Result<(), SessionError> {
    if encoded_state.len() > limits.max_encoded_state_bytes {
        return Err(BoundaryError::limit(
            "INPUT_LIMIT_EXCEEDED",
            limits.max_encoded_state_bytes,
            encoded_state.len(),
        )
        .into());
    }
    Ok(())
}
