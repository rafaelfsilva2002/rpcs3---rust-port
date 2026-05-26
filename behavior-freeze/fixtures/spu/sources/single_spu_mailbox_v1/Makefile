# single_spu_mailbox_v1 — Makefile
# CC0 1.0 (public domain). See LICENSE.md.
#
# Builds:
#   spu/spu_mailbox.elf  -- SPU side, compiled with spu-gcc
#   spu/spu_mailbox.bin  -- raw binary, embedded into the PPU executable
#   spu_mailbox_bin.s    -- assembly stub exposing spu_mailbox_bin / _size symbols
#   single_spu_mailbox_v1.elf  -- PPU executable
#   single_spu_mailbox_v1.self -- signed self for RPCS3
#
# Requires:
#   - $PS3DEV pointing at the toolchain root
#   - $PSL1GHT pointing at PSL1GHT root (typically $PS3DEV/psl1ght)
#   - PATH including $PS3DEV/{ppu,spu}/bin and PSL1GHT host tools (bin2s, fself, etc).

ifndef PS3DEV
$(error PS3DEV is not set. Source /etc/profile.d/ps3dev.sh first.)
endif

ifndef PSL1GHT
PSL1GHT := $(PS3DEV)/psl1ght
endif

# --- toolchain ---
PPU_PREFIX  := powerpc64-ps3-elf
SPU_PREFIX  := spu

PPU_CC      := $(PS3DEV)/ppu/bin/$(PPU_PREFIX)-gcc
PPU_OBJCOPY := $(PS3DEV)/ppu/bin/$(PPU_PREFIX)-objcopy
SPU_CC      := $(PS3DEV)/spu/bin/$(SPU_PREFIX)-gcc
SPU_OBJCOPY := $(PS3DEV)/spu/bin/$(SPU_PREFIX)-objcopy
# PSL1GHT host tools install to $PS3DEV/bin (NOT $PSL1GHT/host/* per the
# ps3toolchain layout; psl1ght's own Makefile installs them as
# `prefix=$(PS3DEV) install`).
BIN2S       := $(PS3DEV)/bin/bin2s
FSELF       := $(PS3DEV)/bin/fself
SPRXLINKER  := $(PS3DEV)/bin/sprxlinker

# --- output names ---
TARGET      := single_spu_mailbox_v1
PPU_ELF     := $(TARGET).elf
PPU_SELF    := $(TARGET).self
SPU_C       := spu/spu_mailbox.c
SPU_ELF     := spu/spu_mailbox.elf
# Embed the FULL SPU ELF (not the raw .bin from objcopy) — sysSpuImageImport
# parses the ELF header + program headers to populate the SPU image.
SPU_S       := spu_mailbox_bin.s
SPU_O       := spu_mailbox_bin.o

# --- flags ---
# SPU side: minimal standalone executable, no startup code.
# `-nostartfiles -nostdlib` skips crt0 + newlib, which would otherwise
# pull in code (ROTQBY etc) outside the iteration-1 SPU interpreter
# subset. Our SPU "main" is the entry point directly; e_entry = 0.
SPU_CFLAGS  := -O2 -Wall -Wno-attributes -nostartfiles -nostdlib
SPU_LDFLAGS := -Wl,--gc-sections -nostartfiles -nostdlib -Wl,--entry,main
# PPU side: PSL1GHT app linked against lv2.ld (PPU LV2 process layout) +
# minimal PSL1GHT runtime libraries needed by sysSpuThreadGroup* + mailbox.
PPU_CFLAGS  := -O2 -Wall -mcpu=cell -I$(PSL1GHT)/ppu/include
PPU_LDFLAGS := -L$(PSL1GHT)/ppu/lib -T $(PS3DEV)/ppu/powerpc64-ps3-elf/lib/lv2.ld \
               -llv2 -lsysmodule -lio -lsysutil -lrt

.PHONY: all clean
all: $(PPU_SELF)

$(SPU_ELF): $(SPU_C)
	@mkdir -p spu
	$(SPU_CC) $(SPU_CFLAGS) -o $@ $< $(SPU_LDFLAGS)

# Embed the SPU ELF directly. Renamed to "spu_mailbox.bin" only to make
# bin2s produce sensibly-named symbols (`spu_mailbox_bin{,_size}`); the
# CONTENTS are the full ELF, which is what sysSpuImageImport parses.
$(SPU_S): $(SPU_ELF)
	cp $< spu_mailbox.bin
	$(BIN2S) spu_mailbox.bin > $@

$(SPU_O): $(SPU_S)
	$(PPU_CC) -c -o $@ $<

$(PPU_ELF): main.c $(SPU_O)
	$(PPU_CC) $(PPU_CFLAGS) -o $@ main.c $(SPU_O) $(PPU_LDFLAGS)
	$(SPRXLINKER) $@ || true

$(PPU_SELF): $(PPU_ELF)
	$(FSELF) $< $@

clean:
	rm -f $(PPU_SELF) $(PPU_ELF) $(SPU_ELF) spu_mailbox.bin $(SPU_S) $(SPU_O)
