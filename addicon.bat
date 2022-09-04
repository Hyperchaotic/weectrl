cargo build --release

rem rc.exe comes with the Windows SDK but it's not in the path.
"C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\rc.exe" resources.rc

cargo rustc --release --example weeapp -- -C link-args="resources.res"

