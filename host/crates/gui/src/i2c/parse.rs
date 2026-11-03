//! Text fields to bytes, with the same rules as the CLI.

use usb_i2c::proto::MAX_XFER;

pub fn parse_addr(s: &str) -> Result<u8, String> {
    let s = s.trim();
    let v = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u8::from_str_radix(h, 16).map_err(|e| format!("地址无效：{e}"))?,
        None => s.parse::<u8>().map_err(|e| format!("地址无效：{e}"))?,
    };
    if v > 0x7F {
        return Err(format!("0x{v:02x} 不是 7 位地址"));
    }
    Ok(v)
}

pub fn parse_bytes(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        let h = tok
            .strip_prefix("0x")
            .or_else(|| tok.strip_prefix("0X"))
            .unwrap_or(tok);
        let b = u8::from_str_radix(h, 16).map_err(|_| format!("无法解析字节 {tok}"))?;
        out.push(b);
    }
    if out.len() > MAX_XFER {
        return Err(format!("单次最多 {MAX_XFER} 字节"));
    }
    Ok(out)
}

pub fn parse_len(s: &str) -> Result<usize, String> {
    let n: usize = s.trim().parse().map_err(|_| format!("读取长度无效：{s}"))?;
    if n == 0 || n > MAX_XFER {
        return Err(format!("读取长度要在 1 到 {MAX_XFER} 之间"));
    }
    Ok(n)
}

pub fn hex_dump(data: &[u8]) -> String {
    let mut out = String::new();
    for (i, row) in data.chunks(16).enumerate() {
        let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
        out.push_str(&format!("{:04x}: {}\n", i * 16, hex.join(" ")));
    }
    out
}

/// `de ad be ef`, what the write field accepts back.
pub fn to_hex(data: &[u8]) -> String {
    let hex: Vec<String> = data.iter().map(|b| format!("{b:02x}")).collect();
    hex.join(" ")
}

/// Short form of a bridge error for a table cell; the full text goes in the tooltip.
pub fn short_error(e: &str) -> &'static str {
    if e.contains("Nack") {
        "NACK"
    } else if e.contains("Timeout") || e.contains("timeout") {
        "超时"
    } else if e == "未连接" {
        "未连接"
    } else {
        "失败"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_matches_cli_rules() {
        assert_eq!(parse_addr("0x50").unwrap(), 0x50);
        assert_eq!(parse_addr("80").unwrap(), 80);
        assert!(parse_addr("0x80").is_err());
        assert!(parse_addr("50").unwrap() != 0x50);
    }

    #[test]
    fn bytes_and_length() {
        assert_eq!(parse_bytes("00 de 0xAD").unwrap(), vec![0x00, 0xde, 0xad]);
        assert!(parse_bytes("  ").unwrap().is_empty());
        assert!(parse_bytes("zz").is_err());
        assert!(parse_bytes(&"00 ".repeat(257)).is_err());
        assert_eq!(parse_len("256").unwrap(), 256);
        assert!(parse_len("0").is_err());
        assert!(parse_len("257").is_err());
    }

    #[test]
    fn dump_rows() {
        let s = hex_dump(&[0x11, 0x22]);
        assert_eq!(s, "0000: 11 22\n");
        assert_eq!(to_hex(&[0xde, 0xad]), "de ad");
        assert_eq!(
            parse_bytes(&to_hex(&[0, 0x7f, 0xff])).unwrap(),
            vec![0, 0x7f, 0xff]
        );
    }

    #[test]
    fn errors_shorten() {
        assert_eq!(short_error("I2C 0x50: Nack (hpm_stat 2003)"), "NACK");
        assert_eq!(short_error("timeout waiting for reply"), "超时");
        assert_eq!(short_error("未连接"), "未连接");
        assert_eq!(short_error("receiving reply: stall"), "失败");
    }
}
