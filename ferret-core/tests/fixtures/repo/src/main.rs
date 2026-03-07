use std::collections::HashMap;

struct Config {
    name: String,
    values: HashMap<String, i32>,
}

fn main() {
    let config = Config {
        name: "ferret".to_string(),
        values: HashMap::new(),
    };
    println!("Running {}", config.name);
}

fn helper_function(x: i32) -> i32 {
    x * 2 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helper() {
        assert_eq!(helper_function(3), 7);
    }
}
