//! Windows paths, wherever they are read.

/// The drive letter of a Windows path such as `C:\Users`, and what follows it.
pub(crate) fn windows_drive(path: &str) -> Option<(char, &str)> {
    let mut chars = path.chars();
    let (letter, colon, separator) = (chars.next()?, chars.next()?, chars.next()?);
    let windows = letter.is_ascii_alphabetic() && colon == ':' && (separator == '\\' || separator == '/');
    windows.then(|| (letter, path.get(2..).unwrap_or_default()))
}
