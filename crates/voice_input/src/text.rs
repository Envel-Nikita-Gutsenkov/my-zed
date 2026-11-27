//! Cleanup and merging of transcription output.
//!
//! Audio is transcribed in independent chunks, and every chunk is fed to the
//! model on its own. Two artefacts follow from that and both are fixed here:
//!
//! * a model can describe non-speech audio (`[BLANK_AUDIO]`, `(music)`,
//!   `♪`), which must not end up in the message;
//! * a chunk boundary can make the model repeat the words it just transcribed
//!   (when a prompt with previous context is passed), so the overlap is removed
//!   instead of being inserted twice.

/// Words at the start of a chunk that may repeat the end of the transcript.
const MAX_OVERLAP_WORDS: usize = 12;


/// Annotations a speech model emits for audio that contains no speech.
const ANNOTATIONS: &[&str] = &[
    "blank_audio",
    "blank audio",
    "blank",
    "silence",
    "music",
    "noise",
    "applause",
    "laughter",
    "inaudible",
    "unintelligible",
    "static",
    "beep",
    "музыка",
    "тишина",
    "шум",
    "смех",
    "аплодисменты",
    "неразборчиво",
];

/// Whole-chunk outputs that models hallucinate on silence or on the tail of a
/// recording. Matching them exactly (ignoring case and punctuation) is safe;
/// matching them inside real speech would not be.
const HALLUCINATIONS: &[&str] = &[
    "thanks for watching",
    "thank you for watching",
    "thanks for watching!",
    "subscribe to my channel",
    "please subscribe",
    "продолжение следует",
    "спасибо за просмотр",
    "спасибо за внимание",
    "субтитры сделал",
    "субтитры создавал",
    "редактор субтитров",
    "корректор",
];

/// Removes non-speech annotations and normalises whitespace in one chunk.
pub fn clean_chunk(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());

    for line in text.lines() {
        let mut line = line.trim();
        // Some models prefix every segment with `>>` when audio is long.
        while let Some(rest) = line.strip_prefix('>') {
            line = rest.trim_start();
        }
        let kept = strip_annotations(line);
        let kept = kept.trim_matches(|character: char| {
            character.is_whitespace() || matches!(character, '♪' | '♫' | '-' | '–' | '—')
        });
        if kept.is_empty() {
            continue;
        }
        if !cleaned.is_empty() {
            cleaned.push(' ');
        }
        cleaned.push_str(kept);
    }

    let cleaned = collapse_whitespace(&cleaned);
    if is_hallucination(&cleaned) {
        return String::new();
    }
    cleaned
}

/// Appends `chunk` to `transcript`, removing a repeated overlap.
///
/// Returns the text that was appended, which is empty when the chunk carried
/// nothing new (silence, or a repetition of what is already there).
pub fn append_chunk(transcript: &mut String, chunk: &str) -> String {
    let chunk = clean_chunk(chunk);
    if chunk.is_empty() {
        return String::new();
    }

    let overlap = overlap_words(transcript, &chunk);
    let start = word_offset(&chunk, overlap);
    let mut addition = chunk[start..].trim_start().to_string();
    if addition.is_empty() {
        return String::new();
    }

    // Chunk boundaries fall mid-sentence, so the model restarts a sentence
    // without a capital while the transcript already ended one. A chunk that
    // repeated the tail of the transcript was treated as a continuation by the
    // model, so it keeps its original casing.
    if overlap == 0 && ends_sentence(transcript) {
        capitalize_first_word(&mut addition);
    }

    let separator = if transcript.is_empty() || addition.starts_with(is_tightening_punctuation) {
        ""
    } else {
        " "
    };
    transcript.push_str(separator);
    transcript.push_str(&addition);
    format!("{separator}{addition}")
}

/// Returns up to `max_chars` characters from the end of `text`, cut at a word
/// boundary so a truncated word is never offered as context.
pub fn prompt_tail(text: &str, max_chars: usize) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return Some(text.to_string());
    }
    let cut = char_count - max_chars;
    let tail: String = text.chars().skip(cut).collect();
    // Only trim when the cut landed inside a word.
    let started_at_word_boundary = text
        .chars()
        .nth(cut - 1)
        .is_some_and(char::is_whitespace);
    if started_at_word_boundary {
        let tail = tail.trim_start();
        return (!tail.is_empty()).then(|| tail.to_string());
    }
    let tail = match tail.find(char::is_whitespace) {
        Some(index) => &tail[index..],
        None => return None,
    };
    let tail = tail.trim_start();
    (!tail.is_empty()).then(|| tail.to_string())
}

/// Number of whole words at the start of `chunk` that repeat the end of
/// `transcript`.
fn overlap_words(transcript: &str, chunk: &str) -> usize {
    let previous: Vec<String> = transcript
        .split_whitespace()
        .map(normalize_word)
        .filter(|word| !word.is_empty())
        .collect();
    let next: Vec<String> = chunk
        .split_whitespace()
        .map(normalize_word)
        .filter(|word| !word.is_empty())
        .take(MAX_OVERLAP_WORDS)
        .collect();

    let limit = previous.len().min(next.len());
    for count in (1..=limit).rev() {
        let tail = &previous[previous.len() - count..];
        if tail == &next[..count] {
            return count;
        }
    }
    0
}

/// Byte offset of the first character of the `index`-th word in `text`.
///
/// Words are counted exactly like `overlap_words` counts them, so
/// punctuation-only tokens do not shift the offset.
fn word_offset(text: &str, index: usize) -> usize {
    if index == 0 {
        return 0;
    }
    let mut words = 0;
    let mut in_word = false;
    for (offset, character) in text.char_indices() {
        if character.is_whitespace() {
            if in_word {
                words += 1;
                in_word = false;
                if words == index {
                    return offset;
                }
            }
        } else if !in_word {
            // Start of a token: skip tokens that carry no word characters.
            let token = text[offset..].split_whitespace().next().unwrap_or_default();
            if normalize_word(token).is_empty() {
                continue;
            }
            in_word = true;
        }
    }
    text.len()
}

fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_tightening_punctuation(character: char) -> bool {
    matches!(character, ',' | '.' | '!' | '?' | ';' | ':' | ')' | ']' | '»')
}

fn is_hallucination(text: &str) -> bool {
    let normalized: String = text
        .chars()
        .filter(|character| !matches!(character, '.' | '!' | '?' | ','))
        .flat_map(char::to_lowercase)
        .collect();
    let normalized = collapse_whitespace(&normalized);
    HALLUCINATIONS.contains(&normalized.as_str())
}

/// Whether the text ends a sentence (possibly inside closing quotes or
/// brackets).
fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(['"', '\'', ')', ']', '»', '”'])
        .ends_with(['.', '!', '?', '…'])
}

/// Capitalises the first letter only when the chunk starts with an ordinary
/// lowercase word. Identifiers (`getUserName`, `x`) and mixed-case tokens are
/// left alone because capitalising them would change their meaning.
fn capitalize_first_word(text: &mut String) {
    let first = match text.split_whitespace().next() {
        Some(first) => first,
        None => return,
    };
    let mut characters = first.chars();
    let Some(first_character) = characters.next() else {
        return;
    };
    if !first_character.is_lowercase() {
        return;
    }
    if !characters.all(|character| {
        character.is_lowercase() || matches!(character, '-' | '\'' | '’')
    }) {
        return;
    }
    if let Some(uppercase) = first_character.to_uppercase().next() {
        let mut uppercased = String::with_capacity(uppercase.len_utf8());
        uppercased.push(uppercase);
        text.replace_range(..first_character.len_utf8(), &uppercased);
    }
}

/// Drops `[...]`, `(...)` and `*...*` groups that only describe sound.
fn strip_annotations(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(start) = rest.find(['[', '(', '*']) {
        result.push_str(&rest[..start]);
        let opener = rest[start..].chars().next().unwrap_or('[');
        let closer = match opener {
            '[' => ']',
            '(' => ')',
            _ => '*',
        };
        let after = &rest[start + opener.len_utf8()..];
        let Some(end) = after.find(closer) else {
            // Unbalanced bracket: keep the text as it is.
            result.push_str(&rest[start..]);
            return result;
        };
        let inner = &after[..end];
        let trailing = &after[end + closer.len_utf8()..];
        let drop = if inner.trim().is_empty() {
            // `()` glued to a word is part of a call such as `getUserName()`.
            let standalone_before = result
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace);
            let standalone_after = trailing
                .chars()
                .next()
                .is_none_or(char::is_whitespace);
            standalone_before && standalone_after
        } else {
            is_annotation(inner)
        };
        if !drop {
            result.push(opener);
            result.push_str(inner);
            result.push(closer);
        }
        rest = trailing;
    }

    result.push_str(rest);
    result
}

fn is_annotation(inner: &str) -> bool {
    let normalized = inner
        .trim()
        .trim_matches(|character: char| {
            character.is_whitespace() || matches!(character, '♪' | '♫' | '*' | '-')
        })
        .to_lowercase();
    normalized.is_empty() || ANNOTATIONS.contains(&normalized.as_str())
}

fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !result.is_empty() {
                result.push(' ');
            }
            pending_space = false;
            result.push(character);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_non_speech_annotations() {
        assert_eq!(clean_chunk("[BLANK_AUDIO]"), "");
        assert_eq!(clean_chunk("(music)"), "");
        assert_eq!(clean_chunk("♪ ♪"), "");
        assert_eq!(clean_chunk("[музыка]"), "");
        assert_eq!(clean_chunk("[BLANK_AUDIO]\nhello"), "hello");
        assert_eq!(clean_chunk("hello\n[BLANK_AUDIO]\nworld"), "hello world");
    }

    #[test]
    fn keeps_meaningful_parentheses() {
        assert_eq!(clean_chunk("f(x) is a function"), "f(x) is a function");
    }

    #[test]
    fn collapses_newlines_and_double_spaces() {
        assert_eq!(clean_chunk("  hello\n\n  world  "), "hello world");
    }

    #[test]
    fn strips_leading_segment_markers() {
        assert_eq!(clean_chunk(">> hello there"), "hello there");
    }

    #[test]
    fn appends_chunks_with_a_single_space() {
        let mut transcript = String::new();
        assert_eq!(append_chunk(&mut transcript, "hello"), "hello");
        assert_eq!(append_chunk(&mut transcript, "world"), " world");
        assert_eq!(transcript, "hello world");
    }

    #[test]
    fn removes_repeated_overlap_between_chunks() {
        let mut transcript = String::from("the quick brown fox");
        assert_eq!(append_chunk(&mut transcript, "brown fox jumps"), " jumps");
        assert_eq!(transcript, "the quick brown fox jumps");
    }

    #[test]
    fn removes_overlap_that_differs_in_punctuation_and_case() {
        let mut transcript = String::from("Сколько стоит, дом?");
        assert_eq!(append_chunk(&mut transcript, "Дом стоит дорого"), " стоит дорого");
    }

    #[test]
    fn ignores_a_chunk_that_only_repeats_the_transcript() {
        let mut transcript = String::from("hello world");
        assert_eq!(append_chunk(&mut transcript, "hello world"), "");
        assert_eq!(transcript, "hello world");
    }

    #[test]
    fn empty_chunk_changes_nothing() {
        let mut transcript = String::from("hello");
        assert_eq!(append_chunk(&mut transcript, "   "), "");
        assert_eq!(transcript, "hello");
    }

    #[test]
    fn punctuation_is_not_preceded_by_a_space() {
        let mut transcript = String::from("hello");
        assert_eq!(append_chunk(&mut transcript, ", world"), ", world");
    }

    #[test]
    fn punctuation_only_tokens_do_not_shift_the_overlap_offset() {
        let mut transcript = String::from("hello world");
        assert_eq!(append_chunk(&mut transcript, ", world again"), " again");
        assert_eq!(transcript, "hello world again");
    }

    #[test]
    fn capitalises_a_new_sentence_across_a_chunk_boundary() {
        let mut transcript = String::from("Готово.");
        assert_eq!(append_chunk(&mut transcript, "проверь тесты"), " Проверь тесты");
        assert_eq!(transcript, "Готово. Проверь тесты");

        let mut transcript = String::from("Done!");
        assert_eq!(append_chunk(&mut transcript, "now commit"), " Now commit");
    }

    #[test]
    fn does_not_capitalise_identifiers() {
        let mut transcript = String::from("Use it.");
        assert_eq!(
            append_chunk(&mut transcript, "getUserName() here"),
            " getUserName() here"
        );
    }

    #[test]
    fn drops_known_hallucinations() {
        assert_eq!(clean_chunk("Спасибо за просмотр."), "");
        assert_eq!(clean_chunk("Thanks for watching!"), "");
        assert_eq!(clean_chunk("thanks for watching the build"), "thanks for watching the build");
    }

    #[test]
    fn prompt_tail_is_cut_at_a_word_boundary() {
        assert_eq!(prompt_tail("hello world", 100).as_deref(), Some("hello world"));
        assert_eq!(prompt_tail("alpha beta gamma", 10).as_deref(), Some("beta gamma"));
        assert_eq!(prompt_tail("averylongword", 4), None);
        assert_eq!(prompt_tail("   ", 10), None);
    }
}
