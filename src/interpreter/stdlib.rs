use crate::interpreter::{Value, ExprResult};

pub fn handle_call(func: &str, args: &[Value]) -> Option<Result<ExprResult, String>> {
    match func {
        "print_str" => {
            if args.len() != 1 {
                return Some(Err("print_str requires exactly 1 argument".to_string()));
            }
            if let Value::String(s) = &args[0] {
                println!("{}", s);
                Some(Ok(ExprResult::Normal(Value::Unit)))
            } else {
                Some(Err("print_str requires a string".to_string()))
            }
        },
        "print_i64" => {
            if args.len() != 1 {
                return Some(Err("print_i64 requires exactly 1 argument".to_string()));
            }
            if let Value::Int(i) = &args[0] {
                println!("{}", i);
                Some(Ok(ExprResult::Normal(Value::Unit)))
            } else {
                Some(Err("print_i64 requires an i64".to_string()))
            }
        },
        "print" => {
            if args.len() != 1 {
                return Some(Err("print requires exactly 1 argument".to_string()));
            }
            println!("{}", args[0]);
            Some(Ok(ExprResult::Normal(Value::Unit)))
        },
        "to_string" => {
            if args.len() != 1 {
                return Some(Err("to_string requires exactly 1 argument".to_string()));
            }
            Some(Ok(ExprResult::Normal(Value::String(format!("{}", args[0])))))
        },
        _ => None
    }
}
