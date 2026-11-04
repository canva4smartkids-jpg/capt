//! Minimal argument parsing (no external dependency) and target syntax.

use capture_core::{CaptureTarget, Rect};
use std::collections::HashMap;

/// Options that take no value.
const FLAGS: &[&str] = &["mic", "system-audio", "no-cursor"];

#[derive(Debug, Default)]
pub struct Args {
    opts: HashMap<String, String>,
    flags: Vec<String>,
    pub positional: Vec<String>,
}

impl Args {
    pub fn parse(argv: &[String]) -> Result<Args, String> {
        let mut a = Args::default();
        let mut i = 0;
        while i < argv.len() {
            let tok = &argv[i];
            if let Some(name) = tok.strip_prefix("--") {
                if let Some((k, v)) = name.split_once('=') {
                    a.opts.insert(k.to_string(), v.to_string());
                } else if FLAGS.contains(&name) {
                    a.flags.push(name.to_string());
                } else {
                    let v = argv.get(i + 1).ok_or_else(|| format!("--{name} needs a value"))?;
                    a.opts.insert(name.to_string(), v.clone());
                    i += 1;
                }
            } else {
                a.positional.push(tok.clone());
            }
            i += 1;
        }
        Ok(a)
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.opts.get(k).map(String::as_str)
    }

    pub fn flag(&self, k: &str) -> bool {
        self.flags.iter().any(|f| f == k)
    }

    pub fn parse_or<T: std::str::FromStr>(&self, k: &str, default: T) -> Result<T, String> {
        match self.get(k) {
            None => Ok(default),
            Some(v) => v.parse().map_err(|_| format!("invalid value '{v}' for --{k}")),
        }
    }
}

fn parse_rect(s: &str) -> Result<Rect, String> {
    let p: Vec<&str> = s.split(',').collect();
    if p.len() != 4 {
        return Err("usage: region rect must be <x>,<y>,<w>,<h>".into());
    }
    let n = |i: usize| p[i].trim().parse::<i64>().map_err(|_| format!("usage: bad number '{}' in region", p[i]));
    let (x, y, w, h) = (n(0)?, n(1)?, n(2)?, n(3)?);
    if w <= 0 || h <= 0 {
        return Err("usage: region width/height must be positive".into());
    }
    Ok(Rect { x: x as i32, y: y as i32, width: w as u32, height: h as u32 })
}

/// `display`, `display:<id>`, `window:<id>`, `device:<id>`, `region:<display>:<x>,<y>,<w>,<h>`,
/// `region:<x>,<y>,<w>,<h>` (primary display).
pub fn parse_target(t: &str, primary: &dyn Fn() -> capture_core::CaptureResult<String>) -> Result<CaptureTarget, String> {
    let prim = || primary().map_err(|e| e.message);
    if t == "display" {
        return Ok(CaptureTarget::Display { display_id: prim()? });
    }
    if let Some(id) = t.strip_prefix("display:") {
        return Ok(CaptureTarget::Display { display_id: id.to_string() });
    }
    if let Some(id) = t.strip_prefix("window:") {
        return Ok(CaptureTarget::Window { window_id: id.to_string() });
    }
    if let Some(id) = t.strip_prefix("device:") {
        return Ok(CaptureTarget::Device { device_id: id.to_string() });
    }
    if let Some(rest) = t.strip_prefix("region:") {
        return match rest.rsplit_once(':') {
            Some((display, rect)) => Ok(CaptureTarget::Region { display_id: display.to_string(), rect: parse_rect(rect)? }),
            None => Ok(CaptureTarget::Region { display_id: prim()?, rect: parse_rect(rest)? }),
        };
    }
    Err(format!("usage: unknown target '{t}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> capture_core::CaptureResult<String> {
        Ok("screen".into())
    }

    #[test]
    fn parses_options_and_flags() {
        let argv: Vec<String> = ["--target", "display:0", "--mic", "--fps=60", "file.mp4"].iter().map(|s| s.to_string()).collect();
        let a = Args::parse(&argv).unwrap();
        assert_eq!(a.get("target"), Some("display:0"));
        assert!(a.flag("mic"));
        assert_eq!(a.parse_or("fps", 30u32).unwrap(), 60);
        assert_eq!(a.positional, vec!["file.mp4"]);
        assert!(Args::parse(&["--out".to_string()]).is_err());
        assert!(a.parse_or::<u32>("target", 1).is_err());
    }

    #[test]
    fn parses_targets() {
        assert_eq!(parse_target("display", &p).unwrap(), CaptureTarget::Display { display_id: "screen".into() });
        assert_eq!(parse_target("device:v4l2:/dev/video0", &p).unwrap(), CaptureTarget::Device { device_id: "v4l2:/dev/video0".into() });
        assert_eq!(parse_target("window:0x1a00003", &p).unwrap(), CaptureTarget::Window { window_id: "0x1a00003".into() });
        assert_eq!(
            parse_target("region:all:10,20,300,200", &p).unwrap(),
            CaptureTarget::Region { display_id: "all".into(), rect: Rect { x: 10, y: 20, width: 300, height: 200 } }
        );
        assert_eq!(
            parse_target("region:5,6,70,80", &p).unwrap(),
            CaptureTarget::Region { display_id: "screen".into(), rect: Rect { x: 5, y: 6, width: 70, height: 80 } }
        );
        assert!(parse_target("region:1,2,3", &p).is_err());
        assert!(parse_target("bogus", &p).is_err());
    }
}
