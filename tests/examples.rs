//! What the project ships must honour its own contract.

use lazy_install::script::contract::TEMPLATE;
use lazy_install::script::validate::check_text;

#[test]
fn the_template_satisfies_the_contract() {
    assert_eq!(check_text(TEMPLATE), Ok(()));
}

#[test]
fn the_example_satisfies_the_contract() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/scripts/kitty.sh"
    ))
    .unwrap();
    assert_eq!(check_text(&text), Ok(()));
}
