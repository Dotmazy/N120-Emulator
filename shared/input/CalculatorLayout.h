#pragma once
#include <QList>
#include <QPainterPath>
#include <QRectF>
#include <QString>

struct CalculatorKey {
    int index;
    QString name;
    QRectF bounds;
};

const QList<CalculatorKey>& calculatorKeys();
QPainterPath calculatorKeyHitShape(const CalculatorKey& key);
int calculatorHitKey(QPointF position);
