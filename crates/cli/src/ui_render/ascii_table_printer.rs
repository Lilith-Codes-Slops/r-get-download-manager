pub struct AsciiTablePrinter;

impl AsciiTablePrinter {
    pub fn print_header(cols: &[&str]) {
        println!("{}", cols.join(" | "));
    }

    pub fn print_row(cols: &[&str]) {
        println!("{}", cols.join(" | "));
    }
}
