# Profile-Guided Optimization (PGO) Build Script for Windows
# Ensure you have the llvm-tools component installed:
# rustup component add llvm-tools

param(
    [switch]$VerboseOutput
)

$CargoFlags = @()
if ($VerboseOutput) {
    $CargoFlags += "--verbose"
}

$PGO_DIR = Join-Path $Pwd "target/pgo-data"
$MERGED_PROFDATA = Join-Path $PGO_DIR "merged.profdata"

# Force Cargo to display build progress bar
$Env:CARGO_TERM_PROGRESS_WHEN = "always"
$Env:CARGO_TERM_PROGRESS_WIDTH = "100"

# 1. Clean previous profile data
Write-Host "Step 1: Cleaning previous profile data in target/pgo-data..." -ForegroundColor Green
if (Test-Path $PGO_DIR) {
    Remove-Item -Recurse -Force $PGO_DIR
}
New-Item -ItemType Directory -Path $PGO_DIR | Out-Null

# 2. Build instrumented binary
Write-Host "Step 2: Building instrumented binary with target-cpu=native..." -ForegroundColor Green
if ($VerboseOutput) {
    Write-Host "Running command: cargo build --profile pgo-gen --verbose" -ForegroundColor Gray
} else {
    Write-Host "Running command: cargo build --profile pgo-gen" -ForegroundColor Gray
}
$Env:RUSTFLAGS = "-C profile-generate=$PGO_DIR -C target-cpu=native"
cargo build --profile pgo-gen @CargoFlags

# 3. Instruction to collect profile data
Write-Host ""
Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "STEP 3: RUN WORKLOAD TO GENERATE PROFILE DATA" -ForegroundColor Cyan
Write-Host "Please run the compiled binary under realistic workloads to gather profiles." -ForegroundColor Cyan
Write-Host "Example: Run the bot and play several tracks, search songs, etc." -ForegroundColor Cyan
Write-Host "The binary is located at: .\target\pgo-gen\serenya.exe" -ForegroundColor Yellow
Write-Host "Press enter when you are done running the bot and want to build the optimized binary." -ForegroundColor Cyan
Write-Host "================================================================================" -ForegroundColor Cyan
Read-Host "Press Enter to continue..."

# 4. Locate llvm-profdata tool
Write-Host "Step 4: Locating llvm-profdata tool..." -ForegroundColor Green
$SysRoot = (rustc --print sysroot)
$TargetTriple = (rustc -vV | Select-String "host:").Line.Split(" ")[1]
$LlvmProfData = Join-Path $SysRoot "lib\rustlib\$TargetTriple\bin\llvm-profdata.exe"

if (-not (Test-Path $LlvmProfData)) {
    # Try finding in the path as fallback
    $LlvmProfData = Get-Command llvm-profdata -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
}

if (-not $LlvmProfData) {
    Write-Error "llvm-profdata not found. Please install llvm-tools using: rustup component add llvm-tools"
    Exit 1
}
Write-Host "Found llvm-profdata at: $LlvmProfData" -ForegroundColor Gray

# 5. Merge profile data
Write-Host "Step 5: Merging profile data..." -ForegroundColor Green
if ($VerboseOutput) {
    Write-Host "Running command: $LlvmProfData merge -o $MERGED_PROFDATA $PGO_DIR" -ForegroundColor Gray
}
& $LlvmProfData merge -o $MERGED_PROFDATA $PGO_DIR

# 6. Build optimized binary using profile data
Write-Host "Step 6: Building optimized binary with target-cpu=native..." -ForegroundColor Green
if ($VerboseOutput) {
    Write-Host "Running command: cargo build --profile pgo-use --verbose" -ForegroundColor Gray
} else {
    Write-Host "Running command: cargo build --profile pgo-use" -ForegroundColor Gray
}
$Env:RUSTFLAGS = "-C profile-use=$MERGED_PROFDATA -C target-cpu=native"
cargo build --profile pgo-use @CargoFlags

Write-Host "Optimized binary built successfully!" -ForegroundColor Green
Write-Host "Location: .\target\pgo-use\serenya.exe" -ForegroundColor Yellow
