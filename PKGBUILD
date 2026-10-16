# Maintainer: Mateusz Kowalczyk
# Temporary API v1 baseline; review a newer source pin before plugin release.
pkgname=ancs-bridge
pkgver=0.1.0
pkgrel=2
pkgdesc='Secure local iPhone notification forwarding for Linux via Apple ANCS'
arch=('x86_64')
url='https://github.com/mateuszkowalczyk/ancs-bridge'
license=('MIT')
options=('!debug')
depends=('bluez' 'dbus' 'wireplumber')
makedepends=('cargo' 'pkgconf')
_commit=c4544e07060245f939f0a2fc0dbe18de10be2ba4
source=("$pkgname-$_commit.tar.gz::$url/archive/$_commit.tar.gz")
sha256sums=('df3361c020b40c9650f2f3b3d24afc7df79b95a5da5963cc1c258dfa47150183')

prepare() {
  cd "$srcdir/$pkgname-$_commit"
  cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
}

build() {
  cd "$srcdir/$pkgname-$_commit"
  cargo build --release --locked --frozen --offline
}

check() {
  cd "$srcdir/$pkgname-$_commit"
  cargo test --all-targets --locked --frozen --offline
}

package() {
  cd "$srcdir/$pkgname-$_commit"
  install -Dm755 target/release/ancs-bridge "$pkgdir/usr/bin/ancs-bridge"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 packaging/ancs-bridge.service \
    "$pkgdir/usr/lib/systemd/user/ancs-bridge.service"
}
