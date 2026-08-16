include $(TOPDIR)/rules.mk

PKG_NAME:=unetic-core
PKG_VERSION:=$(shell sed -n 's/^version = "\([^"]*\)"/\1/p' $(CURDIR)/source/Cargo.toml | head -n1)
PKG_RELEASE:=1
PKG_LICENSE:=GPL-2.0-only
PKG_LICENSE_FILES:=LICENSE
PKG_BUILD_DIR:=$(BUILD_DIR)/$(PKG_NAME)-$(PKG_VERSION)

include $(INCLUDE_DIR)/package.mk

define Package/unetic-core
  SECTION:=admin
  CATEGORY:=Administration
  TITLE:=Unetic control-plane daemon
  URL:=https://github.com/Unetic/unetic-core
  DEPENDS:=+ubus
endef

define Package/unetic-core/description
 Unetic control-plane daemon for OpenWrt.
endef

define Build/Prepare
	mkdir -p $(PKG_BUILD_DIR)
	$(CP) $(CURDIR)/source/. $(PKG_BUILD_DIR)/
endef

define Build/Compile
endef

define Package/unetic-core/install
	$(INSTALL_DIR) $(1)/usr/sbin
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/openwrt-binary $(1)/usr/sbin/unetic-core
	$(INSTALL_DIR) $(1)/etc/init.d
	$(INSTALL_BIN) ./files/unetic-core.init $(1)/etc/init.d/unetic-core
endef

$(eval $(call BuildPackage,unetic-core))
