# single_gcm_draw_v1 — Makefile
# CC0 1.0 (public domain). See LICENSE.md.
#
# R13.3: extend single_gcm_emit_v1 (R13.2) with a real DRAW_ARRAYS via
# rsxDrawVertexArray, so the captured stream decodes to a snapshot
# with a non-empty draw_calls list. Same toolchain as R13.2.

ifndef PS3DEV
$(error PS3DEV is not set.)
endif

ifndef PSL1GHT
PSL1GHT := $(PS3DEV)/psl1ght
endif

PPU_PREFIX  := powerpc64-ps3-elf
PPU_CC      := $(PS3DEV)/ppu/bin/$(PPU_PREFIX)-gcc
FSELF       := $(PS3DEV)/bin/fself
SPRXLINKER  := $(PS3DEV)/bin/sprxlinker

TARGET      := single_gcm_draw_v1
PPU_ELF     := $(TARGET).elf
PPU_SELF    := $(TARGET).self

PPU_CFLAGS  := -O2 -Wall -mcpu=cell -I$(PSL1GHT)/ppu/include
PPU_LDFLAGS := -L$(PSL1GHT)/ppu/lib -T $(PS3DEV)/ppu/powerpc64-ps3-elf/lib/lv2.ld \
               -lrsx -lgcm_sys -llv2 -lsysmodule -lio -lsysutil -lrt

.PHONY: all clean
all: $(PPU_SELF)

$(PPU_ELF): main.c
	$(PPU_CC) $(PPU_CFLAGS) -o $@ main.c $(PPU_LDFLAGS)
	$(SPRXLINKER) $@ || true

$(PPU_SELF): $(PPU_ELF)
	$(FSELF) $< $@

clean:
	rm -f $(PPU_SELF) $(PPU_ELF)
