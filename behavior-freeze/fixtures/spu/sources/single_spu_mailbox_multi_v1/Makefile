# single_spu_mailbox_multi_v1 — Makefile
# CC0 1.0 (public domain). See LICENSE.md.
#
# Builds:
#   spu/spu_mailbox_multi.elf  -- SPU side, compiled with spu-gcc
#   spu_mailbox_multi_bin.s    -- assembly stub exposing
#                                  spu_mailbox_multi_bin / _size symbols
#   single_spu_mailbox_multi_v1.elf   -- PPU executable
#   single_spu_mailbox_multi_v1.self  -- signed self for RPCS3
#
# Requires (same as the other R5.11 fixtures):
#   - $PS3DEV pointing at the toolchain root
#   - $PSL1GHT pointing at PSL1GHT root (typically $PS3DEV/psl1ght)
#   - PATH including $PS3DEV/{ppu,spu}/bin and PSL1GHT host tools
#     (bin2s, fself, sprxlinker).

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
TARGET      := single_spu_mailbox_multi_v1
PPU_ELF     := $(TARGET).elf
PPU_SELF    := $(TARGET).self
SPU_C       := spu/spu_mailbox_multi.c
SPU_ELF     := spu/spu_mailbox_multi.elf
SPU_S       := spu_mailbox_multi_bin.s
SPU_O       := spu_mailbox_multi_bin.o

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
	cp $< spu_mailbox_multi.bin
	$(BIN2S) spu_mailbox_multi.bin > $@

$(SPU_O): $(SPU_S)
	$(PPU_CC) -c -o $@ $<

$(PPU_ELF): main.c $(SPU_O)
	$(PPU_CC) $(PPU_CFLAGS) -o $@ main.c $(SPU_O) $(PPU_LDFLAGS)
	$(SPRXLINKER) $@ || true

$(PPU_SELF): $(PPU_ELF)
	$(FSELF) $< $@

clean:
	rm -f $(PPU_SELF) $(PPU_ELF) $(SPU_ELF) spu_mailbox_multi.bin $(SPU_S) $(SPU_O)
