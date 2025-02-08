/// Holder for current scanner position inside the source file. 
/// Also keeps track of current line and useful offsets 
pub struct PositionInfo {
    /// Beginning position of a lexeme
    pub start: usize,
    /// Where scanner is currently in the file
    pub current: usize,
    /// Current line being processed by scanner
    pub line: usize,
    /// Similar to `current`, but from the start of the line.
    /// This variable is reset each time a newline encountered
    pub current_from_line_start: usize
}

impl PositionInfo {
    pub fn new() -> Self {
        Self {
            start: 0,
            current: 0,
            line: 1,
            current_from_line_start: 0,
        }
    }
    
    pub fn advance(&mut self) {
        self.current += 1;
        self.current_from_line_start += 1;
    }
    
    pub fn advance_line(&mut self) {
        self.line += 1;
        self.current_from_line_start = 0;
    }
}