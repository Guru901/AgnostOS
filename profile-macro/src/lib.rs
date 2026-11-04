//! Small attribute macro for opt-in function invocation profiling.

use proc_macro::{Delimiter, Group, TokenStream, TokenTree};

/// Excludes one function from an enclosing `#[profiled_module]`.
#[proc_macro_attribute]
pub fn not_profiled(_attribute: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Instruments every function body in an external module source file.
///
/// Use on an out-of-line module declaration with an explicit source path:
/// `#[profiled_module("src/console/mod.rs")] mod console;`.
#[proc_macro_attribute]
pub fn profiled_module(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let Some(path) = attribute
        .into_iter()
        .find_map(|token| match token {
            TokenTree::Literal(literal) => Some(literal.to_string()),
            _ => None,
        })
        .filter(|value| value.starts_with('"') && value.ends_with('"'))
    else {
        return compile_error("expected a source path string, e.g. #[profiled_module(\"src/foo.rs\")]");
    };
    let relative_path = path.trim_matches('"');
    let manifest = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(path) => std::path::PathBuf::from(path),
        Err(_) => return compile_error("CARGO_MANIFEST_DIR is unavailable"),
    };
    let source_path = manifest.join(relative_path);
    let source = match std::fs::read_to_string(&source_path) {
        Ok(source) => source,
        Err(error) => {
            return compile_error(&format!(
                "cannot read profiled module {}: {error}",
                source_path.display()
            ));
        }
    };
    let mut source_tokens = match source.parse::<TokenStream>() {
        Ok(tokens) => {
            let module_dir = if source_path.file_name().is_some_and(|name| name == "mod.rs") {
                source_path.parent().unwrap_or(&manifest).to_path_buf()
            } else {
                source_path
                    .parent()
                    .unwrap_or(&manifest)
                    .join(source_path.file_stem().unwrap_or_default())
            };
            let tokens = resolve_external_module_paths(tokens, &module_dir);
            transform_scope(tokens)
        }
        Err(error) => return compile_error(&format!("cannot parse profiled module: {error}")),
    };
    let dependency: TokenStream = format!(
        "const _: &[u8] = include_bytes!({:?});",
        source_path.to_string_lossy()
    )
    .parse()
    .expect("source dependency token stream is valid");
    source_tokens.extend(dependency);

    let output: Vec<_> = strip_path_attributes(item.into_iter().collect());
    let Some(module_index) = output.iter().position(
        |token| matches!(token, TokenTree::Ident(ident) if ident.to_string() == "mod"),
    ) else {
        return compile_error("#[profiled_module] must be applied to a module declaration");
    };
    let Some(TokenTree::Ident(_module_name)) = output.get(module_index + 1) else {
        return compile_error("could not read module name");
    };
    if !output.iter().any(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == ';')) {
        return compile_error("#[profiled_module] expects an out-of-line module declaration");
    }

    let out_dir = manifest.join("target/profiled_modules");
    if let Err(error) = std::fs::create_dir_all(&out_dir) {
        return compile_error(&format!("cannot create generated module directory: {error}"));
    }
    let generated_name: String = relative_path
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect();
    let generated_path = out_dir.join(format!("profiled_{generated_name}"));
    if let Err(error) = std::fs::write(&generated_path, source_tokens.to_string()) {
        return compile_error(&format!("cannot write generated module source: {error}"));
    }
    let path_attribute: TokenStream = format!("#[path = {:?}]", generated_path.to_string_lossy())
        .parse()
        .expect("generated path attribute is valid");
    let mut expanded = path_attribute;
    expanded.extend(output);
    expanded.into_iter().collect()
}

fn strip_path_attributes(tokens: Vec<TokenTree>) -> Vec<TokenTree> {
    let mut output = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        let is_path_attribute = index + 1 < tokens.len()
            && matches!(&tokens[index], TokenTree::Punct(punct) if punct.as_char() == '#')
            && matches!(&tokens[index + 1], TokenTree::Group(group)
                if group.delimiter() == Delimiter::Bracket
                    && group.stream().to_string().starts_with("path"));
        if is_path_attribute {
            index += 2;
        } else {
            output.push(tokens[index].clone());
            index += 1;
        }
    }
    output
}

fn compile_error(message: &str) -> TokenStream {
    format!("compile_error!({message:?});")
        .parse()
        .expect("valid compile_error token stream")
}

fn function_name(tokens: &[TokenTree], index: usize) -> Option<String> {
    if !matches!(tokens.get(index), Some(TokenTree::Ident(ident)) if ident.to_string() == "fn") {
        return None;
    }
    match tokens.get(index + 1) {
        Some(TokenTree::Ident(ident)) => Some(ident.to_string()),
        _ => None,
    }
}

fn has_marker_before(tokens: &[TokenTree], index: usize, marker: &str) -> bool {
    let start = tokens[..index]
        .iter()
        .rposition(|token| {
            matches!(token, TokenTree::Punct(punct) if punct.as_char() == ';')
                || matches!(token, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace)
        })
        .map_or(0, |position| position + 1);
    tokens[start..index].iter().any(|token| {
        matches!(token, TokenTree::Group(group)
            if group.delimiter() == Delimiter::Bracket
                && group.stream().to_string().contains(marker))
    })
}

fn function_body_index(tokens: &[TokenTree], index: usize) -> Option<usize> {
    for (offset, token) in tokens[index + 2..].iter().enumerate() {
        match token {
            TokenTree::Punct(punct) if punct.as_char() == ';' => return None,
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => {
                return Some(index + 2 + offset);
            }
            _ => {}
        }
    }
    None
}

fn is_const_function(tokens: &[TokenTree], index: usize) -> bool {
    let start = tokens[..index]
        .iter()
        .rposition(|token| {
            matches!(token, TokenTree::Punct(punct) if punct.as_char() == ';')
                || matches!(token, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace)
        })
        .map_or(0, |position| position + 1);
    tokens[start..index]
        .iter()
        .any(|token| matches!(token, TokenTree::Ident(ident) if ident.to_string() == "const"))
}

fn transform_scope(stream: TokenStream) -> TokenStream {
    let mut tokens: Vec<_> = stream
        .into_iter()
        .map(|token| match token {
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => {
                let mut transformed = Group::new(Delimiter::Brace, transform_scope(group.stream()));
                transformed.set_span(group.span());
                TokenTree::Group(transformed)
            }
            other => other,
        })
        .collect();

    let function_indices: Vec<_> = (0..tokens.len())
        .filter(|index| function_name(&tokens, *index).is_some())
        .collect();
    for index in function_indices.into_iter().rev() {
        if has_marker_before(&tokens, index, "not_profiled")
            || is_const_function(&tokens, index)
        {
            continue;
        }
        let Some(name) = function_name(&tokens, index) else {
            continue;
        };
        let Some(body_index) = function_body_index(&tokens, index) else {
            continue;
        };
        let TokenTree::Group(original) = &tokens[body_index] else {
            continue;
        };
        let mut body = TokenStream::new();
        if let Ok(hook) = format!(
            "#[cfg(feature = \"profiling\")] let _profile_guard = crate::profiling::enter(concat!(module_path!(), \"::\", \"{name}\"));"
        )
        .parse::<TokenStream>()
        {
            body.extend(hook);
        }
        body.extend(original.stream());
        let mut replacement = Group::new(Delimiter::Brace, body);
        replacement.set_span(original.span());
        tokens[body_index] = TokenTree::Group(replacement);
    }
    tokens.into_iter().collect()
}

fn resolve_external_module_paths(stream: TokenStream, module_dir: &std::path::Path) -> TokenStream {
    let mut tokens: Vec<_> = stream.into_iter().collect();
    let mut index = 0;
    while index + 1 < tokens.len() {
        if !matches!(&tokens[index], TokenTree::Ident(ident) if ident.to_string() == "mod") {
            index += 1;
            continue;
        }
        let Some(TokenTree::Ident(name)) = tokens.get(index + 1) else {
            index += 1;
            continue;
        };
        let name = name.to_string();
        let Some(next) = tokens.get(index + 2) else {
            index += 1;
            continue;
        };
        if let TokenTree::Group(group) = next {
            if group.delimiter() == Delimiter::Brace {
                let child_dir = module_dir.join(&name);
                let nested = resolve_external_module_paths(group.stream(), &child_dir);
                let mut replacement = Group::new(Delimiter::Brace, nested);
                replacement.set_span(group.span());
                tokens[index + 2] = TokenTree::Group(replacement);
            }
            index += 3;
            continue;
        }
        if !matches!(next, TokenTree::Punct(punct) if punct.as_char() == ';') {
            index += 1;
            continue;
        }
        let mut item_start = tokens[..index]
            .iter()
            .rposition(|token| {
                matches!(token, TokenTree::Punct(punct) if punct.as_char() == ';')
                    || matches!(token, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace)
            })
            .map_or(0, |position| position + 1);
        while item_start + 2 < index
            && matches!(&tokens[item_start], TokenTree::Punct(punct) if punct.as_char() == '#')
            && matches!(&tokens[item_start + 1], TokenTree::Punct(punct) if punct.as_char() == '!')
            && matches!(&tokens[item_start + 2], TokenTree::Group(group) if group.delimiter() == Delimiter::Bracket)
        {
            item_start += 3;
        }
        let has_path = tokens[item_start..index].iter().any(|token| {
            matches!(token, TokenTree::Group(group)
                if group.delimiter() == Delimiter::Bracket
                    && group.stream().to_string().starts_with("path"))
        });
        if !has_path {
            let direct = module_dir.join(format!("{name}.rs"));
            let nested = module_dir.join(&name).join("mod.rs");
            let child_path = if direct.is_file() {
                direct
            } else {
                nested
            };
            if child_path.is_file() {
                let attribute: Vec<_> = format!("#[path = {:?}]", child_path.to_string_lossy())
                    .parse::<TokenStream>()
                    .expect("generated child module path is valid")
                    .into_iter()
                    .collect();
                tokens.splice(item_start..item_start, attribute);
                index += 2;
            }
        }
        index += 3;
    }
    tokens.into_iter().collect()
}
