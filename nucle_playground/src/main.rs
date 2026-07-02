use tiny_http::{Server, Response, Header, Method};
use serde::{Deserialize, Serialize};

use nucle_demo_core::{run_benchmark, run_pipeline_demo};

fn main() {
    let host = "127.0.0.1:8080";
    let server = Server::http(host).unwrap();
    println!("NucleScript Playground Server running at http://{}", host);

    // Embed the static HTML directly so that running the binary from any Cwd works
    let index_html = include_str!("../static/index.html");

    for request in server.incoming_requests() {
        match (request.method(), request.url()) {
            (&Method::Get, "/" | "/index.html") => {
                let response = Response::from_string(index_html)
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap());
                let _ = request.respond(response);
            }
            (&Method::Post, "/analyze") => {
                handle_json(request, |body: AnalyzeRequest| {
                    Ok(nucle_lang::playground::analyze_source(&body.source))
                });
            }
            (&Method::Post, "/benchmark") => {
                handle_json(request, run_benchmark);
            }
            (&Method::Post, "/pipeline-demo") => {
                handle_json(request, run_pipeline_demo);
            }
            _ => {
                let response = Response::from_string("Not Found")
                    .with_status_code(404);
                let _ = request.respond(response);
            }
        }
    }
}

/// Read a JSON request body, deserialize it, run `handler`, and write back
/// a JSON response — shared by all three endpoints so each handler only
/// deals with its own request/response shape.
fn handle_json<Req, Res>(
    mut request: tiny_http::Request,
    handler: impl FnOnce(Req) -> Result<Res, String>,
) where
    Req: for<'de> Deserialize<'de>,
    Res: Serialize,
{
    let mut body = String::new();
    if let Err(e) = request.as_reader().read_to_string(&mut body) {
        let response = Response::from_string(format!("Error reading body: {}", e)).with_status_code(400);
        let _ = request.respond(response);
        return;
    }

    let parsed: Req = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            let response = Response::from_string(format!("JSON parse error: {}", e)).with_status_code(400);
            let _ = request.respond(response);
            return;
        }
    };

    match handler(parsed) {
        Ok(result) => {
            let json = serde_json::to_string(&result).unwrap();
            let response = Response::from_string(json)
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
            let _ = request.respond(response);
        }
        Err(e) => {
            let json = serde_json::json!({ "error": e }).to_string();
            let response = Response::from_string(json)
                .with_status_code(400)
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
            let _ = request.respond(response);
        }
    }
}

#[derive(Deserialize)]
struct AnalyzeRequest {
    source: String,
}
