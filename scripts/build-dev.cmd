@echo off
set PATH=C:\Users\12565\.cargo\bin;%PATH%
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cargo %*
