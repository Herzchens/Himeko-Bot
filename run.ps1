Write-Host "=============================================" -ForegroundColor Cyan
Write-Host "   BIÊN DỊCH VÀ TỐI ƯU DUNG LƯỢNG HIMEKO BOT " -ForegroundColor Cyan
Write-Host "=============================================" -ForegroundColor Cyan

# Bước 1: Biên dịch ở chế độ release
Write-Host "[1/3] Đang biên dịch bản Release..." -ForegroundColor Yellow
cargo build --release

if ($LASTEXITCODE -eq 0) {
    Write-Host "[2/3] Biên dịch thành công! Đang sao chép file chạy ra thư mục gốc..." -ForegroundColor Green
    Copy-Item -Path "target\release\himeko-bot.exe" -Destination "himeko-bot.exe" -Force
    
    # Bước 2: Dọn dẹp cache của cargo để giải phóng 7GB dung lượng
    Write-Host "[3/3] Đang dọn dẹp thư mục rác biên dịch (cargo clean)..." -ForegroundColor Yellow
    cargo clean
    
    Write-Host "---------------------------------------------" -ForegroundColor Cyan
    Write-Host "✅ Đã giải phóng xong dung lượng!" -ForegroundColor Green
    Write-Host "🚀 Đang khởi chạy bot..." -ForegroundColor Green
    Write-Host "=============================================" -ForegroundColor Cyan
    
    # Khởi chạy bot
    .\himeko-bot.exe
} else {
    Write-Host "❌ Biên dịch thất bại! Vui lòng kiểm tra lỗi code ở trên." -ForegroundColor Red
}
