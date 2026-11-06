#pragma once
#include "MobileController.h"
#include <QQuickPaintedItem>

class LcdItem : public QQuickPaintedItem {
    Q_OBJECT
    Q_PROPERTY(MobileController* controller READ controller WRITE setController NOTIFY controllerChanged)
  public:
    using QQuickPaintedItem::QQuickPaintedItem;

    MobileController* controller() const {
        return source;
    }

    void setController(MobileController* value);
    void paint(QPainter* painter) override;
  signals:
    void controllerChanged();

  private:
    MobileController* source = nullptr;
};
