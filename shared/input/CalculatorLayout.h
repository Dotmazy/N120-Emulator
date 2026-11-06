#pragma once
#include <QList>
#include <QRectF>
#include <QString>

struct CalculatorKey {
    int index;
    QString name;
    QRectF bounds;
};

const QList<CalculatorKey>& calculatorKeys();
