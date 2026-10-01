.DEFAULT_GOAL := all
.DELETE_ON_ERROR:

DEVKITPRO ?= /opt/devkitpro
DEVKITARM ?= $(DEVKITPRO)/devkitARM
LIBGBA := $(DEVKITPRO)/libgba
CC := $(DEVKITARM)/bin/arm-none-eabi-gcc
OBJCOPY := $(DEVKITARM)/bin/arm-none-eabi-objcopy
GBAFIX := $(DEVKITPRO)/tools/bin/gbafix
HOSTCC ?= cc
PYTHON ?= python3

BUILD ?= build
TARGET := $(BUILD)/rom-dumper_mb
OBJECTS := $(BUILD)/main.o $(BUILD)/dumper.o $(BUILD)/save.o
ARCH := -mcpu=arm7tdmi -mtune=arm7tdmi -mthumb -mthumb-interwork
CFLAGS := $(ARCH) -std=gnu11 -O2 -g -Wall -Wextra -Werror \
          -ffunction-sections -fdata-sections -I$(LIBGBA)/include
LDFLAGS := $(ARCH) -specs=gba_mb.specs -Wl,-Map,$(TARGET).map

.PHONY: all check clean
all: $(TARGET).gba

$(BUILD):
	mkdir -p $@

$(BUILD)/%.o: source/%.c source/dumper.h Makefile | $(BUILD)
	$(CC) $(CFLAGS) -MMD -MP -c $< -o $@

$(TARGET).elf: $(OBJECTS) Makefile
	$(CC) $(LDFLAGS) $(OBJECTS) -L$(LIBGBA)/lib -lgba -o $@
	$(DEVKITARM)/bin/arm-none-eabi-size $@

$(TARGET).gba: $(TARGET).elf tools/check_rom.py
	$(OBJCOPY) -O binary $< $@
	$(GBAFIX) $@ -tROMDUMPER -cRDMP -m00
	$(PYTHON) tools/check_rom.py --pad $@

# Native protocol tests require neither devkitPro nor a GBA.
$(BUILD)/test-dumper: tests/test_dumper.c source/dumper.c source/dumper.h source/save.h Makefile | $(BUILD)
	$(HOSTCC) -std=c11 -O2 -Wall -Wextra -Werror -Isource \
		tests/test_dumper.c source/dumper.c -o $@

$(BUILD)/test-save: tests/test_save.c source/save.c source/save.h Makefile | $(BUILD)
	$(HOSTCC) -std=c11 -O2 -Wall -Wextra -Werror -Isource \
		tests/test_save.c source/save.c -o $@

check: $(BUILD)/test-dumper $(BUILD)/test-save
	./$(BUILD)/test-dumper
	./$(BUILD)/test-save

clean:
	rm -rf $(BUILD)

-include $(OBJECTS:.o=.d)
