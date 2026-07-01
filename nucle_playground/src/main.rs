use tiny_http::{Server, Response, Header, Method};
use serde::Deserialize;

#[derive(Deserialize)]
struct AnalyzeRequest {
    source: String,
}

fn main() {
    let host = "127.0.0.1:8080";
    let server = Server::http(host).unwrap();
    println!("NucleScript Playground Server running at http://{}", host);

    // Embed the static HTML directly so that running the binary from any Cwd works
    let index_html = include_str!("../static/index.html");

    for mut request in server.incoming_requests() {
        match (request.method(), request.url()) {
            (&Method::Get, "/" | "/index.html") => {
                let response = Response::from_string(index_html)
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap());
                let _ = request.respond(response);
            }
            (&Method::Post, "/analyze") => {
                let mut body = String::new();
                if let Err(e) = request.as_reader().read_to_string(&mut body) {
                    let response = Response::from_string(format!("Error reading body: {}", e))
                        .with_status_code(400);
                    let _ = request.respond(response);
                    continue;
                }

                let req: AnalyzeRequest = match serde_json::from_str(&body) {
                    Ok(r) => r,
                    Err(e) => {
                        let response = Response::from_string(format!("JSON parse error: {}", e))
                            .with_status_code(400);
                        let _ = request.respond(response);
                        continue;
                    }
                };

                let report = nucle_lang::playground::analyze_source(&req.source);
                let report_json = serde_json::to_string(&report).unwrap();

                let response = Response::from_string(report_json)
                    .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                let _ = request.respond(response);
            }
            _ => {
                let response = Response::from_string("Not Found")
                    .with_status_code(404);
                let _ = request.respond(response);
            }
        }
    }
}
