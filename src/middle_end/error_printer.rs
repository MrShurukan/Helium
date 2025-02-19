use colored::Colorize;
use crate::frontend::utils::{LocatableInSource, SourceLocation};

// TODO: Refactor that a bit
/// Gets a helper string for error displaying with a marker on every second line.
/// ### Example
/// This function is used to create a string that looks something like this:
/// ```
/// 14. var stuff = (1 + 3 +
///                  *******
/// 15.              13 + "hi" +
///                  ************
/// 16.              19);
///                  **
/// ```
/// Resulting string consists of multiple lines and has a marker denoted by a sequence of
/// `*` on every other line. The lines are parsed from `expression` by the `expression.line_start` and
/// the `expression.line_end` numbers. Lines are fetched from `source` string.
pub fn get_error_marked_line<T: LocatableInSource>(locatable: &T, source: &str) -> String {
    let location = locatable.get_location();
    get_error_marked_line_location(location, source)
}

pub fn get_error_marked_line_location(location: &SourceLocation, source: &str) -> String {
    let mut result_string = String::new();
    for line_number in location.line_start..=location.line_end {
        let line = source.split('\n').nth(line_number - 1);
        if line.is_none() {
            return format!("[Internal Error] Couldn't read line {line_number} from source file");
        }

        // Prep the line itself
        // i.e. `14. var stuff = (1 + 3 +`
        let line = line.unwrap();
        let line_number_string = format!("{line_number}. ");
        result_string += "\n";
        result_string += &line_number_string;
        result_string += line;

        // On the first line we calculate the spaces as an offset to an expression.
        // Marker is calculated until the end of the line unless this line is also the end
        // of the expression
        if line_number == location.line_start {
            let spaces = " ".repeat(line_number_string.len() + location.start_offset);

            if line_number == location.line_end {
                let marker = "*".repeat(location.end_offset - location.start_offset).red();
                result_string += &format!("\n{spaces}{marker}");
                return result_string;
            }

            let marker = "*".repeat(line.len() - location.start_offset).red();
            result_string += &format!("\n{spaces}{marker}");
        }
        // On every other line we skip the whitespace at the beginning
        // and then start drawing the marker. If this is the last line
        // then we draw according to the offset. Otherwise we draw until the end of the line
        else {
            let spaces = " ".repeat(line_number_string.len() +
                line.chars().take_while(char::is_ascii_whitespace).count());

            if line_number == location.line_end {
                let marker = "*".repeat(location.end_offset - spaces.len() + 2).red();
                result_string += &format!("\n{spaces}{marker}");
                return result_string;
            }

            let marker = "*".repeat(line.len() - spaces.len()).red();
            result_string += &format!("\n{spaces}{marker}");
        }
    }


    result_string
}