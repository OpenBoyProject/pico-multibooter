.DEFAULT_GOAL := all
.DELETE_ON_ERROR:

DEVKITPRO ?= /opt/devkitpro
DEVKITARM ?= $(DEVKITPRO)/devkitARM
LIBGBA := $(DEVKITPRO)/libgba
CC := $(DEVKITARM)/bin/arm-none-eabi-gcc
OBJCOPY := $(DEVKITARM)/bin/arm-none-eabi-objcopy
GBAFIX := $(DEVKITPRO)/tools/bin/gbafix
PYTHON ?= python3
CHECK_ROM := ../rom-dumper/tools/check_rom.py

BUILD ?= build
TARGET := $(BUILD)/hello-world_mb
OBJECTS := $(BUILD)/main.o
ARCH := -mcpu=arm7tdmi -mtune=arm7tdmi -mthumb -mthumb-interwork
CFLAGS := $(ARCH) -std=gnu11 -O2 -g -Wall -Wextra -Werror \
          -ffunction-sections -fdata-sections -I$(LIBGBA)/include
LDFLAGS := $(ARCH) -specs=gba_mb.specs -Wl,-Map,$(TARGET).map

.PHONY: all check clean
all: $(TARGET).gba

$(BUILD):
	mkdir -p $@

$(BUILD)/%.o: source/%.c Makefile | $(BUILD)
	$(CC) $(CFLAGS) -MMD -MP -c $< -o $@

$(TARGET).elf: $(OBJECTS) Makefile
	$(CC) $(LDFLAGS) $(OBJECTS) -L$(LIBGBA)/lib -lgba -o $@

$(TARGET).gba: $(TARGET).elf $(CHECK_ROM)
	$(OBJCOPY) -O binary $< $@
	$(GBAFIX) $@ -tHELLOWORLD -cHELO -m00
	$(PYTHON) $(CHECK_ROM) --pad $@

check: $(TARGET).gba
	$(PYTHON) $(CHECK_ROM) $<

clean:
	rm -rf $(BUILD)

-include $(OBJECTS:.o=.d)
