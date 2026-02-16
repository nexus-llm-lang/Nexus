use crate::interpreter::{Value, ExprResult};

pub fn handle_call(func: &str, args: &[Value]) -> Option<Result<ExprResult, String>> {
    match func {
        "printf" => {
            if args.len() != 2 {
                return Some(Err("printf requires exactly 2 arguments (fmt: str, val: i64)".to_string()));
            }
            if let Value::String(fmt) = &args[0] {
                let val_str = match &args[1] {
                    Value::Int(n) => n.to_string(),
                    _ => format!("{:?}", args[1]), // Fallback
                };
                
                // Replace ALL "{}" or just one? Usually printf is sequential.
                // Let's do simple replacement.
                let output = fmt.replacen("{}", &val_str, 1);
                println!("{}", output);
                Some(Ok(ExprResult::Normal(Value::Unit)))
            } else {
                Some(Err("First argument to printf must be a string".to_string()))
            }
        },
        _ => None
    }
}
