export COLUMNS=80
wget https://raw.githubusercontent.com/GiovanniBussi/macports-ci/master/macports-ci
chmod +x ./macports-ci
./macports-ci install
PATH="/opt/local/bin:$PATH"
sudo port install xorg-libXt

find /usr/X11/include
find /opt/local/include
