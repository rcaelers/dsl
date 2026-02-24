#!/usr/bin/env python3
"""Annotate SPI CSV with human-readable descriptions of each command."""

import sys
import csv
from pathlib import Path

# Register names for the CCD AFE (registers 0x00-0x3F are timing/config)
TIMING_REG_NAMES = {
    # CCD clock phase timing
    **{i: f"CCD_CLK_PHASE_{i:02X}" for i in range(0x00, 0x16)},
    0x16: "CCD_CLK_TRANS_A",
    0x17: "CCD_CLK_TRANS_B",
    # Integration / line timing
    **{i: f"LINE_TIMING_{i:02X}" for i in range(0x18, 0x23)},
    # Unused / zero region
    **{i: f"TIMING_{i:02X}" for i in range(0x23, 0x2E)},
    0x2E: "CONFIG_FLAGS_A",
    0x2F: "CONFIG_FLAGS_B",
    # CDS gain
    **{i: f"CDS_GAIN_{i:02X}" for i in range(0x30, 0x36)},
    0x36: "CDS_GAIN_ALT_A",
    0x37: "CDS_GAIN_ALT_B",
    # Clamp/offset
    **{i: f"CLAMP_OFFSET_{i:02X}" for i in range(0x38, 0x3C)},
    # Channel enable
    **{i: f"CHANNEL_CFG_{i:02X}" for i in range(0x3C, 0x40)},
}

CONTROL_REG_NAMES = {
    0x44: "EXT_CFG_44",
    0x45: "EXT_CFG_45",
    0x46: "EXT_CFG_46",
    0x47: "EXT_CFG_47",
    0x50: "PIXEL_START_POS",
    0x51: "PIXEL_END_POS",
    0x52: "LED_EXPOSURE",
    0x53: "LED_TIMING",
    0x56: "LED_CTRL_A",
    0x57: "LED_CTRL_B",
    0x58: "EXT_CFG_58",
    0x59: "EXT_CFG_59",
    0x5A: "EXT_CFG_5A",
    0x5B: "EXT_CFG_5B",
    0x5C: "EXT_CFG_5C",
    0x60: "STREAM_CTRL",
    0x61: "AFE_MODE",
    0x62: "RGB_SEQUENCER",
    0x63: "SYSTEM_MODE",
    0x64: "EXT_CFG_64",
    0x65: "EXT_CFG_65",
    0x66: "EXT_CFG_66",
    0x67: "PGA_CONFIG",
    0x68: "DATA_BUS_CFG",
    0x69: "EXT_CFG_69",
    0x6A: "SCAN_CTRL",
    0x6B: "VREF_BIAS_DAC",
    0x6F: "EXT_CFG_6F",
    0x70: "EXT_CFG_70",
    0x72: "EXT_CFG_72",
    0x78: "RED_GAIN_OFFSET",
    0x79: "GREEN_GAIN_OFFSET",
    0x7A: "BLUE_GAIN_OFFSET",
    0x7E: "EXT_CFG_7E",
    0x7F: "EXT_CFG_7F",
}

SPECIAL_COMMANDS = {
    0x600081: "START pixel stream from CCD",
    0x600080: "Set stream mode config",
    0x600000: "STOP pixel stream",
    0x630001: "BEGIN config sequence",
    0x630002: "BEGIN scan/acquisition mode",
    0x630004: "BEGIN register readback/verify",
    0x630000: "END mode / idle",
    0x6A0000: "Scan line reset/prepare",
    0x68FFFF: "Enable full 16-bit pixel data output",
    0x680100: "Set config-mode data bus",
}

RGB_SEQ_CMDS = {
    0x020E: "setup",
    0x02CE: "red timing",
    0x00C6: "green timing",
    0x0486: "blue timing",
}


def get_reg_name(reg):
    if reg in TIMING_REG_NAMES:
        return TIMING_REG_NAMES[reg]
    if reg in CONTROL_REG_NAMES:
        return CONTROL_REG_NAMES[reg]
    return f"REG_{reg:02X}"


def decode_rgb_gain(reg, value, is_read):
    channel = {0x78: "RED", 0x79: "GREEN", 0x7A: "BLUE"}[reg]
    gain = (value >> 8) & 0xFF
    offset = value & 0xFF
    rw = "read" if is_read else "write"
    return f"{rw} {channel} gain=0x{gain:02X} offset=0x{offset:02X}"


def describe_command(mosi):
    # Check special commands first
    if mosi in SPECIAL_COMMANDS:
        return SPECIAL_COMMANDS[mosi]

    is_read = (mosi >> 23) & 1 == 1
    reg = (mosi >> 16) & 0x7F
    value = mosi & 0xFFFF
    rw = "READ" if is_read else "WRITE"

    # RGB gain/offset calibration
    if reg in (0x78, 0x79, 0x7A):
        return decode_rgb_gain(reg, value, is_read)

    # RGB sequencer FIFO commands
    if reg == 0x62:
        seq_desc = RGB_SEQ_CMDS.get(value, f"cmd=0x{value:04X}")
        return f"{rw} RGB_SEQUENCER {seq_desc}"

    # Timing registers in config mode (upper byte is config mask, lower is phase)
    if reg <= 0x3F and not is_read and (value & 0xC000) != 0 and value != 0xFFFF:
        hi = (value >> 8) & 0xFF
        lo = value & 0xFF
        return f"{rw} {get_reg_name(reg)} config=0x{hi:02X}{lo:02X}"

    # Timing registers in verify mode (write FFFF then read back)
    if reg <= 0x3F and value == 0xFFFF:
        return f"{rw} {get_reg_name(reg)} verify=0xFFFF"

    # Timing registers zeroed
    if reg <= 0x3F and value == 0x0000 and not is_read:
        return f"{rw} {get_reg_name(reg)} = 0x0000 (clear)"

    # Generic register access
    reg_name = get_reg_name(reg)
    return f"{rw} {reg_name} = 0x{value:04X}"


def annotate_file(input_path):
    output_path = input_path.with_stem(input_path.stem + "_annotated")

    with open(input_path, "r") as fin, open(output_path, "w", newline="") as fout:
        reader = csv.reader(fin)
        writer = csv.writer(fout)

        # Header
        header = next(reader)
        header.append("Description")
        writer.writerow(header)

        for row in reader:
            if len(row) < 3:
                row.append("")
                writer.writerow(row)
                continue

            try:
                mosi = int(row[2].strip(), 16)
                desc = describe_command(mosi)
            except (ValueError, IndexError):
                desc = "???"

            row.append(desc)
            writer.writerow(row)

    print(f"Wrote {output_path}")


def main():
    captures_dir = Path(__file__).parent
    csv_files = sorted(captures_dir.glob("*.csv"))
    # Skip already-annotated and _spi output files
    csv_files = [
        f
        for f in csv_files
        if "_annotated" not in f.stem and "_spi" not in f.stem
    ]

    if not csv_files:
        print("No CSV files found in _captures/")
        sys.exit(1)

    for f in csv_files:
        print(f"Processing {f.name}...")
        annotate_file(f)


if __name__ == "__main__":
    main()
