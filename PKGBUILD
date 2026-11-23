# Maintainer: Mateusz Kowalczyk
# Pin the reviewed source commit that precedes the packaging commit.
pkgname=ancs-bridge
pkgver=0.2.0
pkgrel=2
pkgdesc='Secure local iPhone notification forwarding for Linux via Apple ANCS'
arch=('x86_64')
url='https://github.com/mateuszkowalczyk/ancs-bridge'
license=('MIT')
options=('!debug')
depends=('bluez' 'dbus' 'wireplumber')
makedepends=('cargo' 'pkgconf')
_commit=e19e2869441646d72f3a4090270e98e4cf368fc6
source=("$pkgname-$_commit.tar.gz::$url/archive/$_commit.tar.gz")
sha256sums=('41390c4c5c58711c61185956eceb44fb1c639235da8f1de5502f619c3a712e8e')

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
