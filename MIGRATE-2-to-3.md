
* .set -> .header
* 2.x charset feature controled lossy vs encoding_rs.
* No re-exported tls config
* No re-exported cookie API
* No re-exported json macro
* into_string() -> read_to_string()
* native-certs is gone. native roots are always available.
* lossy utf-8 always enabled also when not charset feature
* agent builder
* no retry idempotent (for now)
* no send body charset encoding (for now)
