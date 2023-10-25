// replace placeholder in template with data
pub fn render_template(template: &str, data: &[(&str, &str)]) -> String {
    let mut result = String::from(template);

    for (placeholder, value) in data {
        result = result.replace(placeholder, value);
    }

    result
}

// convert uint to readable format
pub fn readable_uint(int_str: String) -> String {
    let mut s = String::new();
    for (i, char) in int_str.chars().rev().enumerate() {
        if i % 3 == 0 && i != 0 {
            s.insert(0, ',');
        }
        s.insert(0, char);
    }
    return s;
}
