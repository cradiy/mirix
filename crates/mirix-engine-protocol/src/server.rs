use std::io::{BufRead, Write};

use crate::{Engine, TransportError, read_message, write_message};

use crate::{EngineRequest, EngineResponse, select_menu};

/// Serve an engine until the peer closes between messages.
/// A truncated message remains an error rather than a successful shutdown.
pub fn serve(
    engine: &mut dyn Engine,
    reader: &mut impl BufRead,
    writer: &mut impl Write,
) -> Result<(), TransportError> {
    let mut previous_icon = None;
    while !reader.fill_buf()?.is_empty() {
        let request = read_message(reader)?;
        let handshake = matches!(request, EngineRequest::Hello);
        let response = match request {
            EngineRequest::GetConfiguration { locale } => configuration_response(
                engine,
                &locale,
                crate::ActionResult::new(None, crate::EngineOutput::default()),
            ),
            EngineRequest::Configure {
                locale,
                revision,
                event,
            } => {
                let result = if revision != engine.configuration_revision() {
                    Err("configuration changed; refresh before applying".into())
                } else {
                    engine
                        .configuration(&locale)
                        .and_then(|page| page.validate_event(&event))
                        .and_then(|()| engine.configure(&locale, revision, event))
                };
                match result {
                    Ok(result) => configuration_response(engine, &locale, result),
                    Err(message) => EngineResponse::Error { message },
                }
            }
            EngineRequest::Vocabulary { profile, operation } => {
                match engine.vocabulary(profile.as_deref(), operation) {
                    Ok(result) => EngineResponse::VocabularyState {
                        configuration_revision: engine.configuration_revision(),
                        menus: engine.menus(),
                        result: result.value,
                        output: result.output,
                        candidates: engine.candidates().to_vec(),
                        selected: engine.selected_candidate() as u32,
                    },
                    Err(message) => EngineResponse::Error { message },
                }
            }
            EngineRequest::Hello => EngineResponse::Ready {
                configuration_revision: engine.configuration_revision(),
                menus: engine.menus(),
                punctuation_mode: engine.punctuation_mode(),
            },
            EngineRequest::SelectMenu { menu, entry } => match select_menu(engine, &menu, &entry) {
                Ok(effect) => EngineResponse::MenuState {
                    configuration_revision: engine.configuration_revision(),
                    menus: engine.menus(),
                    punctuation_mode: engine.punctuation_mode(),
                    output: effect,
                    candidates: engine.candidates().to_vec(),
                    selected: engine.selected_candidate() as u32,
                },
                Err(message) => EngineResponse::Error { message },
            },
            EngineRequest::SetCandidatePages { ends } => {
                if crate::valid_candidate_pages(engine.candidates().len(), &ends) {
                    engine.set_candidate_pages(ends);
                    EngineResponse::PagesSet
                } else {
                    EngineResponse::Error {
                        message: "invalid candidate pages".into(),
                    }
                }
            }
            EngineRequest::Reset => {
                engine.reset();
                EngineResponse::ResetDone {
                    configuration_revision: engine.configuration_revision(),
                    menus: engine.menus(),
                    punctuation_mode: engine.punctuation_mode(),
                }
            }
            EngineRequest::ProcessKey { event } => EngineResponse::State {
                output: engine.process_key(&event),
                configuration_revision: engine.configuration_revision(),
                menus: engine.menus(),
                punctuation_mode: engine.punctuation_mode(),
                candidates: engine.candidates().to_vec(),
                selected: engine.selected_candidate() as u32,
            },
            EngineRequest::Select { id } => EngineResponse::State {
                output: engine.select(id),
                configuration_revision: engine.configuration_revision(),
                menus: engine.menus(),
                punctuation_mode: engine.punctuation_mode(),
                candidates: engine.candidates().to_vec(),
                selected: engine.selected_candidate() as u32,
            },
        };
        crate::validate_candidates(engine.candidates(), engine.selected_candidate())
            .and_then(|()| response.validate())
            .map_err(|message| {
                TransportError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    message,
                ))
            })?;
        let icon = engine.tray_icon();
        if handshake || icon != previous_icon {
            let response = EngineResponse::IconChanged {
                icon: icon.as_deref().cloned(),
            };
            response.validate().map_err(|message| {
                TransportError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    message,
                ))
            })?;
            write_message(writer, &response)?;
            previous_icon = icon;
        }
        write_message(writer, &response)?;
    }
    Ok(())
}

fn configuration_response(
    engine: &mut dyn Engine,
    locale: &str,
    result: crate::ActionResult<Option<String>>,
) -> EngineResponse {
    match engine.configuration(locale).and_then(|page| {
        page.validate()?;
        Ok(page)
    }) {
        Ok(configuration) => EngineResponse::Configuration {
            configuration,
            message: result.value,
            output: result.output,
            revision: engine.configuration_revision(),
            menus: engine.menus(),
            punctuation_mode: engine.punctuation_mode(),
            candidates: engine.candidates().to_vec(),
            selected: engine.selected_candidate() as u32,
        },
        Err(message) => EngineResponse::Error { message },
    }
}
