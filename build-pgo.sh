#!/usr/bin/env bash
set -e

# Profile-Guided Optimization (PGO) Build Script for Linux
# Ensure llvm-tools is installed:
# rustup component add llvm-tools

VERBOSE_FLAG=""
if [[ "$1" == "--verbose" || "$1" == "-v" ]]; then
    VERBOSE_FLAG="--verbose"
fi

PGO_DIR="$(pwd)/target/pgo-data"
MERGED_PROFDATA="$PGO_DIR/merged.profdata"

# Force Cargo to display build progress bar
export CARGO_TERM_PROGRESS_WHEN="always"
export CARGO_TERM_PROGRESS_WIDTH="100"

# 1. Clean previous profile data
echo -e "\e[32mStep 1: Cleaning previous profile data in target/pgo-data...\e[0m"
rm -rf "$PGO_DIR"
mkdir -p "$PGO_DIR"

# 2. Build instrumented binary
echo -e "\e[32mStep 2: Building instrumented binary with target-cpu=native...\e[0m"
if [ -n "$VERBOSE_FLAG" ]; then
    echo -e "\e[90mRunning command: cargo build --profile pgo-gen --verbose\e[0m"
else
    echo -e "\e[90mRunning command: cargo build --profile pgo-gen\e[0m"
fi
export RUSTFLAGS="-C profile-generate=$PGO_DIR -C target-cpu=native"
cargo build --profile pgo-gen $VERBOSE_FLAG

# 3. Instruction to collect profile data
echo ""
echo -e "\e[36m================================================================================\e[0m"
echo -e "\e[36mSTEP 3: RUN WORKLOAD TO GENERATE PROFILE DATA\e[0m"
echo -e "\e[36mPlease run the compiled binary under realistic workloads to gather profiles.\e[0m"
echo -e "\e[36mExample: Run the bot and play several tracks, search songs, etc.\e[0m"
echo -e "\e[36mThe binary is located at: ./target/pgo-gen/serenya\e[0m"
echo -e "\e[36mPress Enter when you are done running the bot and want to build the optimized binary.\e[0m"
echo -e "\e[36m================================================================================\e[0m"
read -r -p "Press Enter to continue..."

# 4. Locate llvm-profdata tool
echo -e "\e[32mStep 4: Locating llvm-profdata tool...\e[0m"
SYSROOT=$(rustc --print sysroot)
TARGET_TRIPLE=$(rustc -vV | grep host: | awk '{print $2}')
LLVM_PROFDATA="$SYSROOT/lib/rustlib/$TARGET_TRIPLE/bin/llvm-profdata"

if [ ! -f "$LLVM_PROFDATA" ]; then
    # Try finding in the path as fallback
    LLVM_PROFDATA=$(which llvm-profdata || true)
fi

if [ -z "$LLVM_PROFDATA" ] || [ ! -f "$LLVM_PROFDATA" ]; then
    echo -e "\e[31mllvm-profdata not found. Please install llvm-tools using: rustup component add llvm-tools\e[0m"
    exit 1
fi
echo -e "\e[90mFound llvm-profdata at: $LLVM_PROFDATA\e[0m"

# 5. Merge profile data
echo -e "\e[32mStep 5: Merging profile data...\e[0m"
if [ -n "$VERBOSE_FLAG" ]; then
    echo -e "\e[90mRunning command: $LLVM_PROFDATA merge -o $MERGED_PROFDATA $PGO_DIR\e[0m"
fi
"$LLVM_PROFDATA" merge -o "$MERGED_PROFDATA" "$PGO_DIR"

# 6. Build optimized binary using profile data
echo -e "\e[32mStep 6: Building optimized binary with target-cpu=native...\e[0m"
if [ -n "$VERBOSE_FLAG" ]; then
    echo -e "\e[90mRunning command: cargo build --profile pgo-use --verbose\e[0m"
else
    echo -e "\e[90mRunning command: cargo build --profile pgo-use\e[0m"
fi
export RUSTFLAGS="-C profile-use=$MERGED_PROFDATA -C target-cpu=native"
cargo build --profile pgo-use $VERBOSE_FLAG

echo -e "\e[32mOptimized binary built successfully!\e[0m"
echo -e "\e[33mLocation: ./target/pgo-use/serenya\e[0m"
