# single_spu_branch_loop_v1 — Makefile
# CC0 1.0 (public domain). See LICENSE.md.
#
# Builds:
#   spu/spu_branch_loop.elf  -- SPU side, compiled with spu-gcc
#   spu/spu_branch_loop.bin  -- copy of the SPU ELF (embedded into PPU)
#   spu_branch_loop_bin.s    -- assembly stub exposing
#                                spu_branch_loop_bin / _size symbols
#   single_spu_branch_loop_v1.elf  -- PPU executable
#   single_spu_branch_loop_v1.self -- signed self for RPCS3
#
# Same shape as single_spu_mailbox_v1's Makefile — only file names
# and `--entry` target differ. See sibling fixture's Makefile for
# rationale on `-nostartfiles -nostdlib` (skip crt0 / newlib that
# would pull in ROTQBY etc. outside the iteration-1 SPU subset)
# and on embedding the FULL SPU ELF (sysSpuImageImport parses the
# header + program headers).
#
# Requires:
#   - $PS3DEV pointing at the toolchain root
#   - $PSL1GHT pointing at PSL1GHT root (typically $PS3DEV/psl1ght)
#   - PATH including $PS3DEV/{ppu,spu}/bin and PSL1GHT host tools.

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
BIN2S       := $(PS3DEV)/bin/bin2s
FSELF       := $(PS3DEV)/bin/fself
SPRXLINKER  := $(PS3DEV)/bin/sprxlinker

# --- output names ---
TARGET      := single_spu_branch_loop_v1
PPU_ELF     := $(TARGET).elf
PPU_SELF    := $(TARGET).self
SPU_C       := spu/spu_branch_loop.c
SPU_ELF     := spu/spu_branch_loop.elf
SPU_S       := spu_branch_loop_bin.s
SPU_O       := spu_branch_loop_bin.o

# --- flags ---
SPU_CFLAGS  := -O2 -Wall -Wno-attributes -nostartfiles -nostdlib
SPU_LDFLAGS := -Wl,--gc-sections -nostartfiles -nostdlib -Wl,--entry,main
PPU_CFLAGS  := -O2 -Wall -mcpu=cell -I$(PSL1GHT)/ppu/include
PPU_LDFLAGS := -L$(PSL1GHT)/ppu/lib -T $(PS3DEV)/ppu/powerpc64-ps3-elf/lib/lv2.ld \
               -llv2 -lsysmodule -lio -lsysutil -lrt

.PHONY: all clean
all: $(PPU_SELF)

$(SPU_ELF): $(SPU_C)
	@mkdir -p spu
	$(SPU_CC) $(SPU_CFLAGS) -o $@ $< $(SPU_LDFLAGS)

$(SPU_S): $(SPU_ELF)
	cp $< spu_branch_loop.bin
	$(BIN2S) spu_branch_loop.bin > $@

$(SPU_O): $(SPU_S)
	$(PPU_CC) -c -o $@ $<

$(PPU_ELF): main.c $(SPU_O)
	$(PPU_CC) $(PPU_CFLAGS) -o $@ main.c $(SPU_O) $(PPU_LDFLAGS)
	$(SPRXLINKER) $@ || true

$(PPU_SELF): $(PPU_ELF)
	$(FSELF) $< $@

clean:
	rm -f $(PPU_SELF) $(PPU_ELF) $(SPU_ELF) spu_branch_loop.bin $(SPU_S) $(SPU_O)
