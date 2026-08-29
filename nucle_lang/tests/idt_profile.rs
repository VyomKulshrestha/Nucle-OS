use nucle_lang::ast::{PoolState, Profile};
use nucle_lang::hardware::{collect_hardware_requests, RequestType};
use nucle_lang::probabilistic::profile_error_rate_percent;
use nucle_lang::{Lexer, Parser};

fn parse(source: &str) -> nucle_lang::ast::Program {
    let tokens = Lexer::new(source).tokenize().expect("lex error");
    Parser::new(tokens).parse_program().expect("parse error")
}

#[test]
fn idt_profile_aliases_parse_to_the_same_type() {
    for alias in ["IDT", "idt", "IDToPools", "idt-opools"] {
        assert_eq!(Profile::parse(alias), Some(Profile::Idt));
    }
    assert_eq!(Profile::Idt.to_string(), "IDT");
    assert_eq!(profile_error_rate_percent(Profile::Idt), 0.05);
}

#[test]
fn idt_synthesis_typechecks_and_reaches_the_hardware_boundary() {
    let program = parse(
        r#"
        pool archive: DnaPool {
            codec: Ternary,
            redundancy: 3x,
            profile: IDT
        }

        let strands: Pool<IDT, 0.05%> = synthesise archive via IDT confirm hardware
        "#,
    );
    let report = nucle_lang::typeck::check_program(&program);
    assert!(!report.has_errors(), "{report}");

    let requests = collect_hardware_requests(&program);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].profile.as_deref(), Some("IDT"));
    assert!(matches!(
        &requests[0].detail,
        RequestType::Synthesis { profile, .. } if profile == "IDT"
    ));
}

#[test]
fn idt_pool_state_is_preserved_by_parsing() {
    let program =
        parse("let strands: Pool<IDT, 0.05%> = synthesise archive via IDT confirm hardware");
    let declaration = &program.declarations[0];
    let nucle_lang::ast::Declaration::Let(binding) = declaration else {
        panic!("expected a let declaration");
    };
    let nucle_lang::ast::TypeExpr::Pool(pool) = &binding.annotation else {
        panic!("expected a pool annotation");
    };
    assert_eq!(pool.state, PoolState::Profile(Profile::Idt));
}
