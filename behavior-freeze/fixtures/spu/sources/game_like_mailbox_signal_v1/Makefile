# game_like_mailbox_signal_v1 — Makefile
# CC0 1.0 (public domain). See LICENSE.md.
#
# Same conventions as the other R5.11/R6.4b fixtures: PSL1GHT
# toolchain via $(PS3DEV) / $(PSL1GHT). Build via Docker image
# `rpcs3-ps3dev-toolchain:local` from the project root:
#
#   docker run --rm -v "$PWD":/work \
#     -w /work/behavior-freeze/fixtures/spu/sources/game_like_mailbox_signal_v1 \
#     rpcs3-ps3dev-toolchain:local \
#     bash -lc 'make clean && make V=1'

ifndef PS3DEV
$(error PS3DEV is not set. Source /etc/profile.d/ps3dev.sh first.)
endif

ifndef PSL1GHT
PSL1GHT := $(PS3DEV)/psl1ght
endif

PPU_PREFIX  := powerpc64-ps3-elf
SPU_PREFIX  := spu

PPU_CC      := $(PS3DEV)/ppu/bin/$(PPU_PREFIX)-gcc
PPU_OBJCOPY := $(PS3DEV)/ppu/bin/$(PPU_PREFIX)-objcopy
SPU_CC      := $(PS3DEV)/spu/bin/$(SPU_PREFIX)-gcc
SPU_OBJCOPY := $(PS3DEV)/spu/bin/$(SPU_PREFIX)-objcopy
BIN2S       := $(PS3DEV)/bin/bin2s
FSELF       := $(PS3DEV)/bin/fself
SPRXLINKER  := $(PS3DEV)/bin/sprxlinker

TARGET      := game_like_mailbox_signal_v1
PPU_ELF     := $(TARGET).elf
PPU_SELF    := $(TARGET).self
SPU_C       := spu/spu_game_like.c
SPU_ELF     := spu/spu_game_like.elf
SPU_S       := spu_game_like_bin.s
SPU_O       := spu_game_like_bin.o

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
	cp $< spu_game_like.bin
	$(BIN2S) spu_game_like.bin > $@

$(SPU_O): $(SPU_S)
	$(PPU_CC) -c -o $@ $<

$(PPU_ELF): main.c $(SPU_O)
	$(PPU_CC) $(PPU_CFLAGS) -o $@ main.c $(SPU_O) $(PPU_LDFLAGS)
	$(SPRXLINKER) $@ || true

$(PPU_SELF): $(PPU_ELF)
	$(FSELF) $< $@

clean:
	rm -f $(PPU_SELF) $(PPU_ELF) $(SPU_ELF) spu_game_like.bin $(SPU_S) $(SPU_O)
