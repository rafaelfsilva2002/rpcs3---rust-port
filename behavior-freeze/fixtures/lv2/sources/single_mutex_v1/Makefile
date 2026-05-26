# single_mutex_v1 — Makefile
# CC0 1.0 (public domain). See LICENSE.md.
#
# Builds the PPU-only kernel sys_mutex round-trip into a signed
# .self for RPCS3 / our Rust EmuCore::run_self consumer.
#
# Requires:
#   - $PS3DEV pointing at the toolchain root
#   - $PSL1GHT pointing at PSL1GHT root (typically $PS3DEV/psl1ght)
#   - PATH including $PS3DEV/ppu/bin and PSL1GHT host tools (fself,
#     sprxlinker)

ifndef PS3DEV
$(error PS3DEV is not set. Source /etc/profile.d/ps3dev.sh first.)
endif

ifndef PSL1GHT
PSL1GHT := $(PS3DEV)/psl1ght
endif

# --- toolchain ---
PPU_PREFIX  := powerpc64-ps3-elf

PPU_CC      := $(PS3DEV)/ppu/bin/$(PPU_PREFIX)-gcc
PPU_OBJCOPY := $(PS3DEV)/ppu/bin/$(PPU_PREFIX)-objcopy
# PSL1GHT host tools install to $PS3DEV/bin via `prefix=$(PS3DEV) install`.
FSELF       := $(PS3DEV)/bin/fself
SPRXLINKER  := $(PS3DEV)/bin/sprxlinker

# --- output names ---
TARGET      := single_mutex_v1
PPU_ELF     := $(TARGET).elf
PPU_SELF    := $(TARGET).self

# --- flags ---
# No SPU embed: this is a PPU-only kernel sys_mutex test. We still
# link against lv2.ld for the standard PSL1GHT process layout.
PPU_CFLAGS  := -O2 -Wall -mcpu=cell -I$(PSL1GHT)/ppu/include
PPU_LDFLAGS := -L$(PSL1GHT)/ppu/lib -T $(PS3DEV)/ppu/powerpc64-ps3-elf/lib/lv2.ld \
               -llv2 -lsysmodule -lio -lsysutil -lrt

.PHONY: all clean
all: $(PPU_SELF)

$(PPU_ELF): main.c
	$(PPU_CC) $(PPU_CFLAGS) -o $@ main.c $(PPU_LDFLAGS)
	$(SPRXLINKER) $@ || true

$(PPU_SELF): $(PPU_ELF)
	$(FSELF) $< $@

clean:
	rm -f $(PPU_SELF) $(PPU_ELF)
