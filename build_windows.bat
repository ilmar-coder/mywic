@echo off
rem Gera dist\EcoReset.exe. Rode num Windows com Python 3.8+ instalado.
python -m pip install --upgrade pyinstaller || exit /b 1
python -m unittest discover -s . -p "test_*.py" || exit /b 1
python -m PyInstaller --noconfirm --onefile --windowed --name EcoReset ecoreset_app.py || exit /b 1
echo.
echo Pronto: dist\EcoReset.exe
