# single_pngdec_decode_v1 — Makefile (CC0 1.0). cellPngDec pixel-decode fixture.
ifndef PS3DEV
$(error PS3DEV is not set.)
endif
ifndef PSL1GHT
PSL1GHT := $(PS3DEV)/psl1ght
endif
PPU_CC      := $(PS3DEV)/ppu/bin/powerpc64-ps3-elf-gcc
FSELF       := $(PS3DEV)/bin/fself
SPRXLINKER  := $(PS3DEV)/bin/sprxlinker
TARGET      := single_pngdec_decode_v1
PPU_CFLAGS  := -O2 -Wall -mcpu=cell -I$(PSL1GHT)/ppu/include
PPU_LDFLAGS := -L$(PSL1GHT)/ppu/lib -T $(PS3DEV)/ppu/powerpc64-ps3-elf/lib/lv2.ld -lpngdec -llv2 -lsysmodule -lsysutil -lio -lrt
.PHONY: all clean
all: $(TARGET).self
$(TARGET).elf: main.c png_data.h
	$(PPU_CC) $(PPU_CFLAGS) -o $@ main.c $(PPU_LDFLAGS)
	$(SPRXLINKER) $@ || true
$(TARGET).self: $(TARGET).elf
	$(FSELF) $< $@
clean:
	rm -f $(TARGET).self $(TARGET).elf
